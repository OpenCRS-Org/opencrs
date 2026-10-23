//! File probing: what can be read from a creative file's own bytes.
//!
//! Port of `crs/probe.go`. The prober is sans-I/O: it never reads anything
//! itself. [`Prober::next`] says which byte range it needs ([`Step::Need`]),
//! the caller reads it and hands it back with [`Prober::feed`], and the
//! cycle repeats until [`Step::Done`]. That lets a host with only
//! range requests (a browser `File`, an HTTP server) probe a large file
//! without reading it all, e.g. an MP4 with `moov` at the end.
//!
//! Headers are read in windows of up to [`WINDOW`] bytes, and the JPEG
//! segment walk and the MP4 box walk go as far as they can inside each
//! window before asking for more, so runs of JPEG fill bytes or of tiny MP4
//! boxes cost one round trip per window, not one per byte. A probe finishes
//! with what it knows after [`MAX_STEPS`] round trips.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// What could be read from a creative file itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Probe {
    pub sha256: String,
    pub bytes: i64,
    /// Sniffed from the file's signature; empty if unknown.
    pub media_type: String,
    /// `None` if the header couldn't be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pixels: Option<[i64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_s: Option<f64>,
}

/// The next thing a [`Prober`] wants.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Read `len` bytes at `offset` and pass them to [`Prober::feed`]. If the
    /// file ends early, pass the shorter slice.
    Need {
        offset: u64,
        len: u32,
    },
    Done(Probe),
}

const HEAD_LEN: u64 = 32;

/// The most bytes asked for in one [`Step::Need`] while walking JPEG
/// segments or MP4 boxes.
pub const WINDOW: u64 = 4096;

/// The most [`Step::Need`]s one probe makes. Real files need a handful;
/// a file built to need more is reported with what was found so far.
pub const MAX_STEPS: u32 = 10_000;

/// A window starting at `off` that doesn't pass `end`.
fn window(off: u64, end: u64) -> u32 {
    end.saturating_sub(off).min(WINDOW) as u32
}

/// Result of one step of the sibling-box scan (Go's `findBox`).
enum ScanOut {
    Need(u64, u32),
    /// Payload offset and payload length.
    Found(u64, u64),
    NotFound,
}

/// Scans sibling boxes in `[off, end)` for `typ`.
struct Scan {
    off: u64,
    end: u64,
    typ: [u8; 4],
}

impl Scan {
    fn new(start: u64, end: u64, typ: &[u8; 4]) -> Scan {
        Scan {
            off: start,
            end,
            typ: *typ,
        }
    }

    fn poll(&self) -> ScanOut {
        if self.off.saturating_add(8) > self.end {
            ScanOut::NotFound
        } else {
            ScanOut::Need(self.off, window(self.off, self.end))
        }
    }

    /// `data` holds the bytes from `self.off`; walks as many boxes as it
    /// covers. A box header cut short by the end of the file ends the scan;
    /// one cut short by the end of the window asks for the next window.
    fn feed(&mut self, data: &[u8]) -> ScanOut {
        let base = self.off;
        loop {
            if self.off.saturating_add(8) > self.end {
                return ScanOut::NotFound;
            }
            let i = (self.off - base) as usize;
            let b = data.get(i..).unwrap_or(&[]);
            let more = |s: &Scan| {
                if i == 0 { ScanOut::NotFound } else { s.poll() }
            };
            if b.len() < 8 {
                return more(self);
            }
            let size = u32::from_be_bytes(b[0..4].try_into().unwrap()) as u64;
            let ty: [u8; 4] = b[4..8].try_into().unwrap();
            let (size, hl) = match size {
                1 => {
                    if b.len() < 16 {
                        return more(self);
                    }
                    (u64::from_be_bytes(b[8..16].try_into().unwrap()), 16)
                }
                0 => (self.end - self.off, 8),
                s => (s, 8),
            };
            // A size above i64::MAX is negative in Go, hence < hl.
            if size > i64::MAX as u64 || size < hl || size > self.end - self.off {
                return ScanOut::NotFound;
            }
            if ty == self.typ {
                return ScanOut::Found(self.off + hl, size - hl);
            }
            self.off += size;
        }
    }
}

enum ScanKind {
    Moov,
    Mvhd { moov: u64, moov_end: u64 },
    Trak { moov_end: u64 },
    Tkhd { next: u64, moov_end: u64 },
}

enum Phase {
    Head,
    /// Looking for a JPEG marker at `off`.
    Jpeg(u64),
    /// Reading a start-of-frame segment.
    JpegSof,
    Scanning(Scan, ScanKind),
    MvhdRead {
        moov: u64,
        moov_end: u64,
    },
    TkhdRead {
        next: u64,
        moov_end: u64,
    },
    Finished,
}

/// Sans-I/O prober. See the module docs.
pub struct Prober {
    size: u64,
    probe: Probe,
    phase: Phase,
    current: Step,
    steps: u32,
}

impl Prober {
    /// `size` and `sha256` (lower-case hex) describe the whole file; the
    /// caller computes them while reading it however it likes.
    pub fn new(size: u64, sha256: String) -> Prober {
        let probe = Probe {
            sha256,
            bytes: size as i64,
            media_type: String::new(),
            pixels: None,
            duration_s: None,
        };
        let mut p = Prober {
            size,
            probe,
            phase: Phase::Head,
            current: Step::Need { offset: 0, len: 0 },
            steps: 0,
        };
        let len = size.min(HEAD_LEN);
        if len == 0 {
            p.finish();
        } else {
            p.need(0, len as u32);
        }
        p
    }

    /// The range wanted next, or the finished probe. Calling it again
    /// before `feed` repeats the same answer.
    #[allow(clippy::should_implement_trait)] // the API name is fixed by the design
    pub fn next(&mut self) -> Step {
        self.current.clone()
    }

    /// Supplies the bytes for the last [`Step::Need`]; shorter than asked
    /// means the file ended. Ignored once done.
    pub fn feed(&mut self, bytes: &[u8]) {
        match std::mem::replace(&mut self.phase, Phase::Finished) {
            Phase::Finished => {}
            Phase::Head => self.on_head(bytes),
            Phase::Jpeg(off) => self.on_jpeg_window(off, bytes),
            Phase::JpegSof => {
                if bytes.len() >= 9 {
                    let h = u16::from_be_bytes([bytes[5], bytes[6]]) as i64;
                    let w = u16::from_be_bytes([bytes[7], bytes[8]]) as i64;
                    self.probe.pixels = Some([w, h]);
                }
                self.finish();
            }
            Phase::Scanning(mut scan, kind) => {
                let out = scan.feed(bytes);
                self.on_scan(scan, kind, out);
            }
            Phase::MvhdRead { moov, moov_end } => {
                if bytes.len() >= 32 {
                    let (timescale, dur) = if bytes[0] == 1 {
                        (
                            u32::from_be_bytes(bytes[20..24].try_into().unwrap()),
                            u64::from_be_bytes(bytes[24..32].try_into().unwrap()),
                        )
                    } else {
                        (
                            u32::from_be_bytes(bytes[12..16].try_into().unwrap()),
                            u32::from_be_bytes(bytes[16..20].try_into().unwrap()) as u64,
                        )
                    };
                    if timescale > 0 {
                        self.probe.duration_s = Some(dur as f64 / timescale as f64);
                    }
                }
                self.start_scan(
                    Scan::new(moov, moov_end, b"trak"),
                    ScanKind::Trak { moov_end },
                );
            }
            Phase::TkhdRead { next, moov_end } => {
                if let Some(px) = tkhd_size(bytes) {
                    self.probe.pixels = Some(px);
                    self.finish();
                } else {
                    self.start_scan(
                        Scan::new(next, moov_end, b"trak"),
                        ScanKind::Trak { moov_end },
                    );
                }
            }
        }
    }

    fn need(&mut self, offset: u64, len: u32) {
        if self.steps >= MAX_STEPS {
            return self.finish();
        }
        self.steps += 1;
        self.current = Step::Need { offset, len };
    }

    fn finish(&mut self) {
        self.phase = Phase::Finished;
        self.current = Step::Done(self.probe.clone());
    }

    fn on_head(&mut self, head: &[u8]) {
        if head.starts_with(b"\x89PNG\r\n\x1a\n") {
            self.probe.media_type = "image/png".into();
            if head.len() >= 24 {
                let w = u32::from_be_bytes(head[16..20].try_into().unwrap()) as i64;
                let h = u32::from_be_bytes(head[20..24].try_into().unwrap()) as i64;
                self.probe.pixels = Some([w, h]);
            }
            self.finish();
        } else if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
            self.probe.media_type = "image/jpeg".into();
            self.jpeg_at(2);
        } else if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
            self.probe.media_type = "image/gif".into();
            if head.len() >= 10 {
                let w = u16::from_le_bytes([head[6], head[7]]) as i64;
                let h = u16::from_le_bytes([head[8], head[9]]) as i64;
                self.probe.pixels = Some([w, h]);
            }
            self.finish();
        } else if head.len() >= 12 && &head[4..8] == b"ftyp" {
            self.probe.media_type = if &head[8..12] == b"qt  " {
                "video/quicktime"
            } else {
                "video/mp4"
            }
            .into();
            self.start_scan(Scan::new(0, self.size, b"moov"), ScanKind::Moov);
        } else {
            self.finish();
        }
    }

    /// Go's `jpegSize` loop head: asks for the window at `off`.
    fn jpeg_at(&mut self, off: u64) {
        if off + 4 > self.size {
            self.finish();
        } else {
            self.phase = Phase::Jpeg(off);
            self.need(off, window(off, self.size));
        }
    }

    /// Walks JPEG markers in a window that starts at `base`, skipping fill
    /// bytes and segments, until it finds a frame header, the end of the
    /// headers, or the end of the window.
    fn on_jpeg_window(&mut self, base: u64, buf: &[u8]) {
        let mut off = base;
        loop {
            if off + 4 > self.size {
                return self.finish();
            }
            let i = (off - base) as usize;
            let b = buf.get(i..).unwrap_or(&[]);
            if b.len() < 4 {
                // Cut short by the end of the file, or of the window.
                return if i == 0 {
                    self.finish()
                } else {
                    self.jpeg_at(off)
                };
            }
            if b[0] != 0xFF {
                return self.finish();
            }
            let marker = b[1];
            if marker == 0xFF {
                // Fill byte.
                off += 1;
                continue;
            }
            let seg_len = u16::from_be_bytes([b[2], b[3]]) as u64;
            let is_sof = (0xC0..=0xCF).contains(&marker)
                && marker != 0xC4
                && marker != 0xC8
                && marker != 0xCC;
            if is_sof {
                if b.len() >= 9 || i == 0 {
                    // At the start of a window, a short read is the file's end.
                    self.phase = Phase::JpegSof;
                    return self.feed(&b[..b.len().min(9)]);
                }
                self.phase = Phase::JpegSof;
                return self.need(off, 9);
            }
            if marker == 0xDA || marker == 0xD9 {
                return self.finish();
            }
            off += 2 + seg_len;
        }
    }

    fn start_scan(&mut self, scan: Scan, kind: ScanKind) {
        let out = scan.poll();
        self.on_scan(scan, kind, out);
    }

    fn on_scan(&mut self, scan: Scan, kind: ScanKind, out: ScanOut) {
        match (kind, out) {
            (kind, ScanOut::Need(off, len)) => {
                self.phase = Phase::Scanning(scan, kind);
                self.need(off, len);
            }
            // No moov: nothing to report.
            (ScanKind::Moov, ScanOut::NotFound) => self.finish(),
            (ScanKind::Moov, ScanOut::Found(moov, len)) => {
                let moov_end = moov + len;
                self.start_scan(
                    Scan::new(moov, moov_end, b"mvhd"),
                    ScanKind::Mvhd { moov, moov_end },
                );
            }
            (ScanKind::Mvhd { moov, moov_end }, ScanOut::Found(mvhd, l)) if l >= 32 => {
                self.phase = Phase::MvhdRead { moov, moov_end };
                self.need(mvhd, 32);
            }
            (ScanKind::Mvhd { moov, moov_end }, _) => {
                self.start_scan(
                    Scan::new(moov, moov_end, b"trak"),
                    ScanKind::Trak { moov_end },
                );
            }
            (ScanKind::Trak { .. }, ScanOut::NotFound) => self.finish(),
            (ScanKind::Trak { moov_end }, ScanOut::Found(trak, l)) => {
                let next = trak + l;
                self.start_scan(
                    Scan::new(trak, next, b"tkhd"),
                    ScanKind::Tkhd { next, moov_end },
                );
            }
            (ScanKind::Tkhd { next, moov_end }, ScanOut::Found(tkhd, tl)) if tl >= 84 => {
                self.phase = Phase::TkhdRead { next, moov_end };
                self.need(tkhd, tl.min(96) as u32);
            }
            (ScanKind::Tkhd { next, moov_end }, _) => {
                self.start_scan(
                    Scan::new(next, moov_end, b"trak"),
                    ScanKind::Trak { moov_end },
                );
            }
        }
    }
}

/// Display size from a track header, if non-zero.
fn tkhd_size(b: &[u8]) -> Option<[i64; 2]> {
    let at = if b.first() == Some(&1) { 88 } else { 76 }; // width/height offset
    if b.len() < at + 8 {
        return None;
    }
    let w = (u32::from_be_bytes(b[at..at + 4].try_into().unwrap()) >> 16) as i64;
    let h = (u32::from_be_bytes(b[at + 4..at + 8].try_into().unwrap()) >> 16) as i64;
    (w > 0 && h > 0).then_some([w, h])
}

/// Hashes `data`, then probes it by driving a [`Prober`] over the slice.
pub fn probe_bytes(data: &[u8]) -> Probe {
    let digest = Sha256::digest(data);
    let mut hex = String::with_capacity(64);
    for b in digest {
        hex.push_str(&format!("{b:02x}"));
    }
    let mut prober = Prober::new(data.len() as u64, hex);
    loop {
        match prober.next() {
            Step::Done(p) => return p,
            Step::Need { offset, len } => {
                let start = (offset as usize).min(data.len());
                let end = (offset as usize)
                    .saturating_add(len as usize)
                    .min(data.len());
                prober.feed(&data[start..end]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = include_bytes!("../tests/fixtures/64x48.png");
    const JPG: &[u8] = include_bytes!("../tests/fixtures/64x48.jpg");
    const GIF: &[u8] = include_bytes!("../tests/fixtures/64x48.gif");

    #[test]
    fn probe_images() {
        for (name, data, mt) in [
            ("png", PNG, "image/png"),
            ("jpg", JPG, "image/jpeg"),
            ("gif", GIF, "image/gif"),
        ] {
            let p = probe_bytes(data);
            assert_eq!(p.media_type, mt, "{name}");
            assert_eq!(p.pixels, Some([64, 48]), "{name}");
            assert_eq!(p.bytes, data.len() as i64, "{name}");
            assert_eq!(p.sha256.len(), 64, "{name}");
            assert_eq!(p.duration_s, None, "{name}");
        }
    }

    fn be32(v: u32) -> [u8; 4] {
        v.to_be_bytes()
    }

    fn mp4_box(typ: &str, payload: &[&[u8]]) -> Vec<u8> {
        let body: Vec<u8> = payload.concat();
        let mut b = be32(8 + body.len() as u32).to_vec();
        b.extend_from_slice(typ.as_bytes());
        b.extend(body);
        b
    }

    /// moov with a v0 mvhd (10 s at timescale 1000) and a video track
    /// header of 1080 x 1920.
    fn synthetic_moov() -> Vec<u8> {
        let mut mvhd = vec![0u8; 100];
        mvhd[12..16].copy_from_slice(&be32(1000));
        mvhd[16..20].copy_from_slice(&be32(10000));
        let mut tkhd = vec![0u8; 84];
        tkhd[76..80].copy_from_slice(&be32(1080 << 16));
        tkhd[80..84].copy_from_slice(&be32(1920 << 16));
        mp4_box(
            "moov",
            &[
                &mp4_box("mvhd", &[&mvhd]),
                &mp4_box("trak", &[&mp4_box("tkhd", &[&tkhd])]),
            ],
        )
    }

    fn ftyp() -> Vec<u8> {
        mp4_box("ftyp", &[b"isom\x00\x00\x02\x00isomavc1"])
    }

    #[test]
    fn probe_mp4() {
        let data = [ftyp(), synthetic_moov()].concat();
        let p = probe_bytes(&data);
        assert_eq!(p.media_type, "video/mp4");
        assert_eq!(p.pixels, Some([1080, 1920]));
        assert_eq!(p.duration_s, Some(10.0));
    }

    #[test]
    fn probe_quicktime_brand() {
        let data = [
            mp4_box("ftyp", &[b"qt  \x00\x00\x00\x00"]),
            synthetic_moov(),
        ]
        .concat();
        assert_eq!(probe_bytes(&data).media_type, "video/quicktime");
    }

    #[test]
    fn probe_mp4_v1_headers() {
        let mut mvhd = vec![0u8; 112];
        mvhd[0] = 1;
        mvhd[20..24].copy_from_slice(&be32(600));
        mvhd[24..32].copy_from_slice(&900u64.to_be_bytes());
        let mut tkhd = vec![0u8; 104];
        tkhd[0] = 1;
        tkhd[88..92].copy_from_slice(&be32(640 << 16));
        tkhd[92..96].copy_from_slice(&be32(360 << 16));
        let moov = mp4_box(
            "moov",
            &[
                &mp4_box("mvhd", &[&mvhd]),
                &mp4_box("trak", &[&mp4_box("tkhd", &[&tkhd])]),
            ],
        );
        let p = probe_bytes(&[ftyp(), moov].concat());
        assert_eq!(p.pixels, Some([640, 360]));
        assert_eq!(p.duration_s, Some(1.5));
    }

    #[test]
    fn probe_unknown_and_truncated() {
        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("a.txt", b"hello".to_vec()),
            ("b.jpg", vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00]),
            (
                "c.mp4",
                [
                    mp4_box("ftyp", &[b"isom"]),
                    vec![0, 0, 0, 99, b'm', b'o', b'o', b'v'],
                ]
                .concat(),
            ),
            ("d.png", b"\x89PNG\r\n\x1a\n".to_vec()),
            ("e.empty", vec![]),
        ];
        for (name, data) in cases {
            let p = probe_bytes(&data);
            assert_eq!(p.pixels, None, "{name}");
            assert_eq!(p.duration_s, None, "{name}");
            assert_eq!(p.bytes, data.len() as i64, "{name}");
        }
        assert_eq!(probe_bytes(b"\x89PNG\r\n\x1a\n").media_type, "image/png");
    }

    #[test]
    fn largesize_box_is_walked() {
        // mdat with a 64-bit size sits before moov.
        let mut mdat = be32(1).to_vec();
        mdat.extend_from_slice(b"mdat");
        mdat.extend_from_slice(&(16u64 + 5).to_be_bytes());
        mdat.extend_from_slice(&[0; 5]);
        let p = probe_bytes(&[ftyp(), mdat, synthetic_moov()].concat());
        assert_eq!(p.pixels, Some([1080, 1920]));
    }

    #[test]
    fn size_zero_box_extends_to_end() {
        // A final box with size 0 that is not moov: no moov found.
        let mut tail = be32(0).to_vec();
        tail.extend_from_slice(b"mdat");
        tail.extend_from_slice(&[0; 20]);
        let p = probe_bytes(&[ftyp(), tail].concat());
        assert_eq!(p.media_type, "video/mp4");
        assert_eq!(p.pixels, None);
    }

    #[test]
    fn jpeg_skips_segments_and_fill_bytes() {
        // SOI, fill byte, APP0 (len 4), SOF0 (17 x 9).
        let mut d = vec![0xFF, 0xD8, 0xFF, 0xFF, 0xE0, 0x00, 0x04, 0, 0];
        d.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x0B, 8, 0, 9, 0, 17, 1, 1, 0x11, 0]);
        let p = probe_bytes(&d);
        assert_eq!(p.pixels, Some([17, 9]));
    }

    #[test]
    fn moov_at_end_reads_only_needed_ranges() {
        const MDAT: usize = 50 * 1024 * 1024;
        let mut mdat = be32((8 + MDAT) as u32).to_vec();
        mdat.extend_from_slice(b"mdat");
        mdat.resize(8 + MDAT, 0xAB);
        let data = [ftyp(), mdat, synthetic_moov()].concat();

        let mut prober = Prober::new(data.len() as u64, "0".repeat(64));
        let mut requested = 0u64;
        let mut max_end = 0u64;
        let probe = loop {
            match prober.next() {
                Step::Done(p) => break p,
                Step::Need { offset, len } => {
                    requested += len as u64;
                    max_end = max_end.max(offset + len as u64);
                    let s = offset as usize;
                    let e = (s + len as usize).min(data.len());
                    prober.feed(&data[s..e]);
                }
            }
        };
        assert_eq!(probe.duration_s, Some(10.0));
        assert_eq!(probe.pixels, Some([1080, 1920]));
        assert!(requested < 64 * 1024, "requested {requested} bytes");
        assert_eq!(max_end, data.len() as u64, "should have reached the tail");
    }

    /// Drives a prober over `data`, counting the round trips.
    fn steps(data: &[u8]) -> (Probe, u32) {
        let mut prober = Prober::new(data.len() as u64, "0".repeat(64));
        let mut n = 0;
        loop {
            match prober.next() {
                Step::Done(p) => return (p, n),
                Step::Need { offset, len } => {
                    n += 1;
                    let s = (offset as usize).min(data.len());
                    let e = (s + len as usize).min(data.len());
                    prober.feed(&data[s..e]);
                }
            }
        }
    }

    /// The review's M4: every JPEG fill byte cost a round trip.
    #[test]
    fn jpeg_fill_bytes_are_skipped_a_window_at_a_time() {
        let mut d = vec![0xFF, 0xD8];
        d.resize(2 + (1 << 20), 0xFF);
        d.extend_from_slice(&[0xC0, 0x00, 0x0B, 8, 0, 9, 0, 17, 1, 1, 0x11, 0]);
        let (p, n) = steps(&d);
        assert_eq!(p.pixels, Some([17, 9]));
        assert!(n <= 2 + (1 << 20) / WINDOW as u32 + 2, "{n} round trips");
    }

    /// The review's M4: every 8-byte MP4 box cost a round trip.
    #[test]
    fn tiny_boxes_are_walked_a_window_at_a_time() {
        let free = mp4_box("free", &[]);
        let data = [ftyp(), free.repeat(100_000), synthetic_moov()].concat();
        let (p, n) = steps(&data);
        assert_eq!(p.pixels, Some([1080, 1920]));
        assert!(n < 300, "{n} round trips");
    }

    #[test]
    fn round_trips_are_capped() {
        // Each segment jumps just past the window, so each needs a trip.
        let mut d = vec![0xFF, 0xD8];
        let seg = WINDOW as usize;
        for _ in 0..(MAX_STEPS as usize + 50) {
            let mut s = vec![0xFF, 0xE0];
            s.extend_from_slice(&((seg - 2) as u16).to_be_bytes());
            s.resize(seg, 0);
            d.extend(s);
        }
        let (p, n) = steps(&d);
        assert_eq!(n, MAX_STEPS);
        assert_eq!(p.media_type, "image/jpeg");
        assert_eq!(p.pixels, None);
    }

    /// The probe as first ported: Go's walk over a whole slice, one marker
    /// or box header at a time.
    fn reference(d: &[u8]) -> (String, Option<[i64; 2]>, Option<f64>) {
        let n = d.len();
        let be32 = |o: usize| u32::from_be_bytes(d[o..o + 4].try_into().unwrap());
        let head = &d[..n.min(32)];
        if head.starts_with(b"\x89PNG\r\n\x1a\n") {
            let px = (n >= 24).then(|| [be32(16) as i64, be32(20) as i64]);
            return ("image/png".into(), px, None);
        }
        if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
            let mut off = 2;
            let mut px = None;
            while off + 4 <= n {
                let b = &d[off..off + 4];
                if b[0] != 0xFF {
                    break;
                }
                if b[1] == 0xFF {
                    off += 1;
                    continue;
                }
                let m = b[1];
                if (0xC0..=0xCF).contains(&m) && m != 0xC4 && m != 0xC8 && m != 0xCC {
                    if off + 9 <= n {
                        let h = u16::from_be_bytes([d[off + 5], d[off + 6]]) as i64;
                        let w = u16::from_be_bytes([d[off + 7], d[off + 8]]) as i64;
                        px = Some([w, h]);
                    }
                    break;
                }
                if m == 0xDA || m == 0xD9 {
                    break;
                }
                off += 2 + u16::from_be_bytes([b[2], b[3]]) as usize;
            }
            return ("image/jpeg".into(), px, None);
        }
        if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
            let px = (n >= 10).then(|| {
                [
                    u16::from_le_bytes([d[6], d[7]]) as i64,
                    u16::from_le_bytes([d[8], d[9]]) as i64,
                ]
            });
            return ("image/gif".into(), px, None);
        }
        if !(n >= 12 && &d[4..8] == b"ftyp") {
            return (String::new(), None, None);
        }
        let mt = if &d[8..12] == b"qt  " {
            "video/quicktime"
        } else {
            "video/mp4"
        };
        let find = |start: usize, end: usize, typ: &[u8]| -> Option<(usize, usize)> {
            let mut off = start;
            while off + 8 <= end {
                if off + 8 > n {
                    return None;
                }
                let (mut size, mut hl) = (be32(off) as u64, 8u64);
                let ty = &d[off + 4..off + 8];
                if size == 1 {
                    if off + 16 > n {
                        return None;
                    }
                    size = u64::from_be_bytes(d[off + 8..off + 16].try_into().unwrap());
                    hl = 16;
                } else if size == 0 {
                    size = (end - off) as u64;
                }
                if size > i64::MAX as u64 || size < hl || size > (end - off) as u64 {
                    return None;
                }
                if ty == typ {
                    return Some((off + hl as usize, (size - hl) as usize));
                }
                off += size as usize;
            }
            None
        };
        let read = |o: usize, l: usize| &d[o.min(n)..(o + l).min(n)];
        let (mut px, mut dur) = (None, None);
        if let Some((moov, ml)) = find(0, n, b"moov") {
            let moov_end = moov + ml;
            if let Some((mvhd, l)) = find(moov, moov_end, b"mvhd")
                && l >= 32
            {
                let b = read(mvhd, 32);
                if b.len() >= 32 {
                    let (ts, du) = if b[0] == 1 {
                        (
                            u32::from_be_bytes(b[20..24].try_into().unwrap()),
                            u64::from_be_bytes(b[24..32].try_into().unwrap()),
                        )
                    } else {
                        (
                            u32::from_be_bytes(b[12..16].try_into().unwrap()),
                            u32::from_be_bytes(b[16..20].try_into().unwrap()) as u64,
                        )
                    };
                    if ts > 0 {
                        dur = Some(du as f64 / ts as f64);
                    }
                }
            }
            let mut off = moov;
            while let Some((trak, l)) = find(off, moov_end, b"trak") {
                let next = trak + l;
                if let Some((tkhd, tl)) = find(trak, next, b"tkhd")
                    && tl >= 84
                    && let Some(s) = tkhd_size(read(tkhd, tl.min(96)))
                {
                    px = Some(s);
                    break;
                }
                off = next;
            }
        }
        (mt.into(), px, dur)
    }

    /// A small deterministic generator (xorshift64).
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    fn random_jpeg(r: &mut Rng) -> Vec<u8> {
        let mut d = vec![0xFF, 0xD8];
        for _ in 0..r.below(12) {
            match r.below(6) {
                0 => d.resize(d.len() + r.below(9000) as usize, 0xFF),
                1 => d.push(r.next() as u8),
                2 => {
                    let m = [0xC0, 0xC2, 0xC4, 0xC8, 0xCF][r.below(5) as usize];
                    d.extend_from_slice(&[0xFF, m, 0, 11, 8]);
                    d.extend_from_slice(&(r.next() as u32).to_be_bytes());
                }
                3 => d.extend_from_slice(&[0xFF, [0xDA, 0xD9][r.below(2) as usize]]),
                _ => {
                    let len = r.below(6000) as usize + 2;
                    d.extend_from_slice(&[0xFF, 0xE1]);
                    d.extend_from_slice(&(len as u16).to_be_bytes());
                    d.resize(d.len() + len - 2, r.next() as u8);
                }
            }
        }
        d
    }

    fn random_boxes(r: &mut Rng, depth: u32) -> Vec<u8> {
        let mut out = Vec::new();
        for _ in 0..r.below(6) {
            let ty = [b"free", b"mdat", b"moov", b"trak", b"mvhd", b"tkhd"][r.below(6) as usize];
            let mut body = if depth < 3 && r.below(2) == 0 {
                random_boxes(r, depth + 1)
            } else {
                let mut b = vec![0u8; r.below(140) as usize];
                for x in b.iter_mut() {
                    *x = if r.below(3) == 0 { r.next() as u8 } else { 0 };
                }
                if b.len() > 84 && r.below(2) == 0 {
                    b[76..80].copy_from_slice(&be32(640 << 16));
                    b[80..84].copy_from_slice(&be32(360 << 16));
                }
                if b.len() > 20 && r.below(2) == 0 {
                    b[12..16].copy_from_slice(&be32(1000));
                }
                b
            };
            match r.below(10) {
                0 => {
                    // 64-bit size.
                    let mut h = be32(1).to_vec();
                    h.extend_from_slice(ty);
                    h.extend_from_slice(&(16 + body.len() as u64).to_be_bytes());
                    out.extend(h);
                    out.append(&mut body);
                }
                1 => {
                    let mut h = be32(r.next() as u32 % 200).to_vec(); // any size
                    h.extend_from_slice(ty);
                    out.extend(h);
                    out.append(&mut body);
                }
                2 if r.below(4) == 0 => {
                    let mut h = be32(0).to_vec(); // to the end
                    h.extend_from_slice(ty);
                    out.extend(h);
                    out.append(&mut body);
                    return out;
                }
                3 => out.resize(out.len() + 8 * r.below(700) as usize, 0), // padding boxes
                _ => out.extend(mp4_box(std::str::from_utf8(ty).unwrap(), &[&body])),
            }
        }
        out
    }

    #[test]
    fn windowed_walk_matches_the_step_by_step_one() {
        let mut r = Rng(0x9E37_79B9_7F4A_7C15);
        for case in 0..4000 {
            let mut d = match case % 3 {
                0 => random_jpeg(&mut r),
                1 => [ftyp(), random_boxes(&mut r, 0)].concat(),
                _ => {
                    // A plausible moov with noise around its boxes.
                    let mut mvhd = vec![0u8; 100];
                    mvhd[12..16].copy_from_slice(&be32(1000));
                    mvhd[16..20].copy_from_slice(&be32(r.below(99999) as u32));
                    let mut tkhd = vec![0u8; 84 + r.below(3) as usize * 10];
                    tkhd[76..80].copy_from_slice(&be32((r.below(4000) as u32) << 16));
                    tkhd[80..84].copy_from_slice(&be32((r.below(4000) as u32) << 16));
                    let trak = mp4_box(
                        "trak",
                        &[&random_boxes(&mut r, 3), &mp4_box("tkhd", &[&tkhd])],
                    );
                    let moov = mp4_box(
                        "moov",
                        &[
                            &random_boxes(&mut r, 3),
                            &mp4_box("mvhd", &[&mvhd]),
                            &random_boxes(&mut r, 3),
                            &trak,
                            &trak,
                        ],
                    );
                    [ftyp(), random_boxes(&mut r, 2), moov].concat()
                }
            };
            if r.below(3) == 0 && !d.is_empty() {
                let cut = r.below(d.len() as u64) as usize;
                d.truncate(cut);
            }
            let p = probe_bytes(&d);
            let (mt, px, dur) = reference(&d);
            assert_eq!(
                (p.media_type.as_str(), p.pixels, p.duration_s),
                (mt.as_str(), px, dur),
                "case {case}: {d:02x?}"
            );
        }
    }

    #[test]
    fn next_repeats_until_fed() {
        let mut p = Prober::new(100, "0".repeat(64));
        assert_eq!(p.next(), p.next());
        assert_eq!(p.next(), Step::Need { offset: 0, len: 32 });
    }

    #[test]
    fn serialises_with_wasm_names() {
        let p = probe_bytes(PNG);
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["pixels"], serde_json::json!([64, 48]));
        assert_eq!(v["media_type"], "image/png");
        assert!(v.get("duration_s").is_none());
        assert!(v["sha256"].is_string() && v["bytes"].is_number());
    }
}
