//! The command line, as clap sees it: commands, flags, and the help text.
//!
//! The old Go-style spellings (`-spec`, `-json=false`, `-allow-local-http`)
//! still work: [`normalize`] rewrites them to `--long` forms before clap
//! parses, so they never appear in help, the man page or completions.

use clap::builder::styling::{AnsiColor, Effects, Styles};
use clap::{Arg, ArgAction, Args, Command, CommandFactory, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

/// Where to read more, shown at the end of the top-level help and in the man
/// page.
pub const DOCS_URL: &str = "https://opencrs.dev/spec/0.1/";

// The exit-status sentence at the end of each command's help. Macros, so
// they can go inside `concat!`.
macro_rules! exit_check {
    () => {
        "0 passed, 1 a file failed, 2 usage or read error"
    };
}
macro_rules! exit_schema {
    () => {
        "0 all valid, 1 a document failed, 2 usage or read error"
    };
}
macro_rules! exit_resolve {
    () => {
        "0 resolved, 1 reference not found, 2 usage or read error"
    };
}
macro_rules! exit_fetch {
    () => {
        "0 all trusted, 1 a document was not trusted, 2 usage or fetch error"
    };
}
macro_rules! exit_conformance {
    () => {
        "0 conforms, 1 a case failed, 2 usage or read error"
    };
}
macro_rules! exit_probe {
    () => {
        "0 every file read, 2 usage or read error"
    };
}
macro_rules! exit_calc {
    () => {
        "0 worked out, 2 usage error (inputs that make no sense)"
    };
}
macro_rules! exit_rule_packs {
    () => {
        "0 shown, 1 no such rule pack, 2 usage error"
    };
}
macro_rules! exit_publisher {
    () => {
        "0 no errors, 1 the check found errors, 2 usage error or the index couldn't be fetched"
    };
}

pub const EXIT_CHECK: &str = exit_check!();
pub const EXIT_SCHEMA: &str = exit_schema!();
pub const EXIT_RESOLVE: &str = exit_resolve!();
pub const EXIT_FETCH: &str = exit_fetch!();
pub const EXIT_CONFORMANCE: &str = exit_conformance!();
pub const EXIT_PROBE: &str = exit_probe!();
pub const EXIT_CALC: &str = exit_calc!();
pub const EXIT_RULE_PACKS: &str = exit_rule_packs!();
pub const EXIT_PUBLISHER: &str = exit_publisher!();

const STYLES: Styles = Styles::styled()
    .header(AnsiColor::Green.on_default().effects(Effects::BOLD))
    .usage(AnsiColor::Green.on_default().effects(Effects::BOLD))
    .literal(AnsiColor::Cyan.on_default().effects(Effects::BOLD))
    .placeholder(AnsiColor::Cyan.on_default())
    .error(AnsiColor::Red.on_default().effects(Effects::BOLD))
    .valid(AnsiColor::Cyan.on_default())
    .invalid(AnsiColor::Yellow.on_default());

/// A heading in the same style clap gives "Usage:" and "Options:". The codes
/// are dropped with the rest of the styling when output isn't a terminal.
macro_rules! heading {
    ($s:literal) => {
        concat!("\x1b[1m\x1b[32m", $s, "\x1b[0m")
    };
}

macro_rules! examples {
    ($($line:literal),+ $(,)?) => {
        concat!(heading!("Examples:"), $("\n  ", $line),+)
    };
}

macro_rules! exit_status {
    ($s:expr) => {
        concat!(heading!("Exit status:"), " ", $s)
    };
}

#[derive(Parser)]
#[command(
    name = "opencrs",
    version,
    about = "Check digital-signage creative against the screens it's made for",
    disable_help_flag = true,
    disable_version_flag = true,
    disable_help_subcommand = true,
    arg_required_else_help = true,
    styles = STYLES,
    after_help = concat!(
        examples!(
            "opencrs check --spec retail.screens.json creative.json",
            "opencrs fetch harbourline.example",
            "opencrs schema screens.json creative.json",
            "opencrs probe lunch-portrait.png",
        ),
        "\n\n",
        heading!("Docs:"),
        " https://opencrs.dev/spec/0.1/",
    ),
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Cmd,
}

#[derive(Subcommand)]
pub enum Cmd {
    #[command(
        about = "Check a creative's files against the screens they target",
        long_about = "Check a creative's files against the screens they target\n\n\
            Reads a creative.json, finds each file it lists, and checks the file's type, \
            size, duration and layout against every screen it targets in the screens.json \
            files you give. Each file's pixel size, byte size and SHA-256 come from the \
            file itself; the manifest's declared values are used only for a file that is \
            missing or when --no-files is set.",
        after_help = exit_status!(exit_check!()),
        after_long_help = concat!(
            examples!(
                "opencrs check --spec retail.screens.json creative.json",
                "opencrs check -s retail.screens.json -s airports.screens.json creative.json",
                "opencrs check --spec retail.screens.json --json creative.json > reports.json",
                "opencrs check --spec retail.screens.json --now 2026-10-22T12:00:00Z creative.json",
            ),
            "\n\n",
            exit_status!(exit_check!()),
        ),
    )]
    Check(CheckArgs),
    #[command(
        about = "Check documents against the OpenCRS schemas",
        long_about = "Check documents against the OpenCRS schemas\n\n\
            Each file is checked against the JSON Schema for its kind (index, screens, \
            creative, report or rule-pack), detected from the document unless --kind is \
            given. For a screens.json it also reports references to formats or venues that \
            don't exist and rules that inheritance has made impossible to meet. A JSON \
            array is checked element by element, so the output of check --json works.",
        after_help = exit_status!(exit_schema!()),
        after_long_help = concat!(
            examples!(
                "opencrs schema screens.json creative.json",
                "opencrs schema --kind index .well-known/opencrs.json",
                "opencrs schema --allow-local-http index.json",
            ),
            "\n\n",
            exit_status!(exit_schema!()),
        ),
    )]
    Schema(SchemaArgs),
    #[command(
        about = "Show the requirements for a format or a screen",
        long_about = "Show the requirements for a format or a screen\n\n\
            Prints the requirements that apply to one format or one screen after \
            inheritance (publisher, network, venue, format, screen) as JSON. A reference \
            is <publisher>/<network>/<format> or <publisher>/<network>/screen/<id>.\n\n\
            With --strictest, prints one set of requirements instead, the strictest of every \
            screen the reference covers: a file that meets it passes on all of them.",
        after_help = exit_status!(exit_resolve!()),
        after_long_help = concat!(
            examples!(
                "opencrs resolve retail.screens.json harbourline.example/retail/portrait-totem-75",
                "opencrs resolve retail.screens.json harbourline.example/retail/screen/HC-L1-002",
                "opencrs resolve --strictest retail.screens.json harbourline.example/retail/portrait-totem-75",
            ),
            "\n\n",
            exit_status!(exit_resolve!()),
        ),
    )]
    Resolve(ResolveArgs),
    #[command(
        about = "Find a publisher's specs and check which to trust",
        long_about = "Find a publisher's specs and check which to trust\n\n\
            Given a domain, reads the publisher's index at /.well-known/opencrs.json, \
            fetches each document it lists, follows delegations, and marks each document \
            trusted, expired, rejected, skipped (a partner document without a token) or \
            failed, with the reason. Given a document URL, checks that its publisher \
            vouches for that exact URL.\n\n\
            Only HTTPS is used, and addresses on private networks are refused. \
            See the NETWORK section of the man page for the limits.",
        after_help = exit_status!(exit_fetch!()),
        after_long_help = concat!(
            examples!(
                "opencrs fetch harbourline.example",
                "opencrs fetch --output specs harbourline.example",
                "opencrs fetch --token harbourline.example=TOKEN --output specs harbourline.example",
                "opencrs fetch https://specs.cms-a.example/harbourline/airports.screens.json",
                "opencrs fetch --allow-local-http localhost:8080",
            ),
            "\n\n",
            exit_status!(exit_fetch!()),
        ),
    )]
    Fetch(FetchArgs),
    #[command(
        about = "Check a publisher's published specs the way a media owner would",
        long_about = "Check a publisher's published specs the way a media owner would\n\n\
            Given a domain, fetches the publisher's index at /.well-known/opencrs.json and \
            every document it lists, follows delegations, and reports which documents are \
            trusted, what is wrong with each (schema errors, broken references, rule \
            conflicts), and how each file is served compared with what the spec asks of \
            publishers: Cache-Control and ETag headers, compression, a JSON content type. \
            Where fetch stops at the trust decisions, publisher gives the advice a \
            publisher needs before announcing its specs.\n\n\
            The same fetching and limits as fetch: only HTTPS, addresses on private \
            networks are refused, and no proxy is used. No tokens are sent, so partner \
            documents are reported as skipped. See the NETWORK section of the man page.",
        after_help = exit_status!(exit_publisher!()),
        after_long_help = concat!(
            examples!(
                "opencrs publisher harbourline.example",
                "opencrs publisher --json harbourline.example > publisher.json",
                "opencrs publisher --now 2026-10-22T12:00:00Z harbourline.example",
            ),
            "\n\n",
            exit_status!(exit_publisher!()),
        ),
    )]
    Publisher(PublisherArgs),
    #[command(
        about = "Show what the validator reads from a file",
        long_about = "Show what the validator reads from a file\n\n\
            Prints each file's type (from its signature, not its name), pixel size, size in \
            bytes, SHA-256 and, for video, duration: the values check compares with a \
            screen's requirements. Only the parts of a large file that are needed are read, \
            after hashing it. A file that is not a type the validator knows is shown with \
            type unknown.",
        after_help = exit_status!(exit_probe!()),
        after_long_help = concat!(
            examples!(
                "opencrs probe lunch-portrait.png",
                "opencrs probe lunch-portrait.png lunch-portrait-logo-right.png",
                "opencrs probe --json creative/*.png > probes.json",
            ),
            "\n\n",
            exit_status!(exit_probe!()),
        ),
    )]
    Probe(ProbeArgs),
    #[command(
        about = "Work out minimum text sizes for a viewing distance",
        long_about = "Work out minimum text sizes for a viewing distance\n\n\
            Works out, as the legibility check does, the smallest text that can be read \
            from the far end of the viewing range (key text) and the near end (legal \
            text). Text is taken to be legible at 150 times its height unless --ratio \
            says otherwise. Give the screen's width in millimetres and pixels to get sizes \
            in pixels, and a dwell time to get the number of words that can be read in it.",
        after_help = exit_status!(exit_calc!()),
        after_long_help = concat!(
            examples!(
                "opencrs legibility --near 2 --far 6 --screen-width-mm 940 --screen-width-px 1080",
                "opencrs legibility --near 2 --far 6 --dwell 4",
                "opencrs legibility --near 3 --far 10 --ratio 250 --json",
            ),
            "\n\n",
            exit_status!(exit_calc!()),
        ),
    )]
    Legibility(LegibilityArgs),
    #[command(
        about = "Work out what a driver can read of a sign at a speed",
        long_about = "Work out what a driver can read of a sign at a speed\n\n\
            Works out, as CRS-GLANCE-TEXT does, how long a driver at a speed takes to \
            cross the viewing range, how much of it is one glance, how many words that \
            reads, and where reading must start. With --pack, also shows the limits \
            each built-in rule pack sets at that speed. List the packs with rule-packs.",
        after_help = exit_status!(exit_calc!()),
        after_long_help = concat!(
            examples!(
                "opencrs dwell --speed 80 --near 20 --far 150",
                "opencrs dwell --speed 80 --near 20 --far 150 --pack au-nsw-transport-corridor@2017-11",
                "opencrs dwell --speed 60 --near 20 --far 150 --screen-width-mm 6000 --screen-width-px 1920 --json",
            ),
            "\n\n",
            exit_status!(exit_calc!()),
        ),
    )]
    Dwell(DwellArgs),
    #[command(
        name = "rule-packs",
        about = "List the built-in rule packs, or show one",
        long_about = "List the built-in rule packs, or show one\n\n\
            Rule packs encode published regulatory rules, such as a state's roadside \
            signage guidelines, so a screens.json can name them. Without a reference, \
            lists the packs built into this tool with their status. With one, \
            name@version, shows that pack's limits: its creative and playback rules, \
            and where each comes from. --json gives the pack as the rule-pack document \
            it is, or the list in the shape the WebAssembly module and the language \
            bindings give.",
        after_help = exit_status!(exit_rule_packs!()),
        after_long_help = concat!(
            examples!(
                "opencrs rule-packs",
                "opencrs rule-packs au-nsw-transport-corridor@2017-11",
                "opencrs rule-packs --json au-nsw-transport-corridor@2017-11",
            ),
            "\n\n",
            exit_status!(exit_rule_packs!()),
        ),
    )]
    RulePacks(RulePacksArgs),
    #[command(
        about = "Run the conformance suite against a validator",
        long_about = "Run the conformance suite against a validator\n\n\
            Runs the cases in the conformance directory against this tool's own validator, \
            or against another validator given with --cmd. The command is run through the \
            shell with each case directory as its last argument and must print the reports \
            as a JSON array, or for a case with strictest expectations, the strictest \
            requirements of each as a JSON array. Cases for strictest requirements are \
            optional: a validator that doesn't offer them is tested with --no-strictest.",
        after_help = exit_status!(exit_conformance!()),
        after_long_help = concat!(
            examples!(
                "opencrs conformance",
                "opencrs conformance --level core --verbose",
                "opencrs conformance --cmd ./my-adapter.sh",
            ),
            "\n\n",
            exit_status!(exit_conformance!()),
        ),
    )]
    Conformance(ConformanceArgs),
    #[command(
        about = "Print shell completions",
        long_about = "Print shell completions\n\n\
            Prints a completion script for the shell to standard output. Save it where \
            your shell looks for completions, or load it from your shell's startup file.",
        after_long_help = concat!(
            examples!(
                "opencrs completion bash > ~/.local/share/bash-completion/completions/opencrs",
                "opencrs completion zsh > ~/.zfunc/_opencrs",
                "opencrs completion fish > ~/.config/fish/completions/opencrs.fish",
                "opencrs completion powershell | Out-String | Invoke-Expression",
            ),
        ),
    )]
    Completion(CompletionArgs),
    #[command(about = "Show the tool, spec and rule pack versions")]
    Version,
}

#[derive(Args)]
pub struct CheckArgs {
    /// The creative.json to check
    #[arg(value_name = "CREATIVE")]
    pub creative: String,
    /// A screens.json to check against, repeatable
    #[arg(
        short,
        long,
        value_name = "SCREENS",
        required = true,
        action = ArgAction::Append,
        allow_hyphen_values = true,
        long_help = "A screens.json to check against. Give it more than once to check \
            against several publishers."
    )]
    pub spec: Vec<String>,
    /// Directory holding the creative files
    #[arg(
        long,
        value_name = "DIR",
        allow_hyphen_values = true,
        long_help = "Directory holding the creative files. Defaults to the manifest's \
            directory."
    )]
    pub files: Option<String>,
    /// Use the manifest's declared values, don't read the files
    #[arg(long = "no-files")]
    pub no_files: bool,
    /// Print the reports as a JSON array
    #[arg(
        long,
        long_help = "Print the reports as a JSON array of spec reports, in the spec's \
            report format."
    )]
    pub json: bool,
    /// Validation time, RFC 3339 (default: now)
    #[arg(long, value_name = "TIME", allow_hyphen_values = true)]
    pub now: Option<String>,
    /// Warn below this multiple of the minimum text size
    #[arg(
        long,
        value_name = "RATIO",
        default_value = "1.25",
        value_parser = parse_float,
        allow_hyphen_values = true,
        long_help = "Warn when text is below this multiple of its minimum size. \
            Zero means the default, 1.25."
    )]
    pub margin: f64,
}

#[derive(Args)]
pub struct SchemaArgs {
    /// The documents to check
    #[arg(value_name = "FILE", required = true, num_args = 1..)]
    pub files: Vec<String>,
    /// Document kind (default: detect it)
    #[arg(
        long,
        value_name = "KIND",
        allow_hyphen_values = true,
        long_help = "Document kind: index, screens, creative, report or rule-pack. \
            Detected from the document when omitted."
    )]
    pub kind: Option<String>,
    /// Accept http://localhost document URLs in an index
    #[arg(long = "allow-local-http")]
    pub allow_local_http: bool,
}

#[derive(Args)]
pub struct ResolveArgs {
    /// The screens.json to read
    #[arg(value_name = "SCREENS")]
    pub screens: String,
    /// A format or screen reference
    #[arg(
        value_name = "REF",
        long_help = "A format or screen reference: <publisher>/<network>/<format> or \
            <publisher>/<network>/screen/<id>."
    )]
    pub reference: String,
    /// Combine every screen the reference covers into one set of requirements
    #[arg(
        long,
        long_help = "Combine every screen the reference covers into one set of \
            requirements that a file can meet to pass on all of them: the largest safe-area \
            insets, the union of blocked regions, the largest minimum text size in pixels \
            for each role, the smallest maximums, and the file types every screen accepts. \
            `sources` names the screen that set each value and `notes` says where no single \
            file can pass everywhere."
    )]
    pub strictest: bool,
}

#[derive(Args)]
pub struct FetchArgs {
    /// A publisher's domain, or the URL of one document
    #[arg(value_name = "DOMAIN|URL")]
    pub target: String,
    /// Bearer token for a publisher's partner documents, repeatable
    #[arg(
        long,
        value_name = "DOMAIN=TOKEN",
        action = ArgAction::Append,
        allow_hyphen_values = true,
        long_help = "Bearer token a publisher issued for its partner documents, as \
            DOMAIN=TOKEN. Give it more than once for several publishers. The token \
            is sent only to that domain."
    )]
    pub token: Vec<String>,
    /// Save trusted documents under this directory
    #[arg(
        short,
        long,
        value_name = "DIR",
        alias = "o",
        allow_hyphen_values = true,
        long_help = "Save trusted documents, exactly as served, under this directory. \
            The files are ready for check --spec. When partner documents are included, \
            a merged.screens.json is written too."
    )]
    pub output: Option<String>,
    /// Print the result as JSON
    #[arg(long)]
    pub json: bool,
    /// Allow http:// and connections to localhost, for testing
    #[arg(
        long = "allow-local-http",
        long_help = "Local testing: allow http:// and connections to localhost, \
            127.0.0.1 and [::1]. Other internal addresses stay refused."
    )]
    pub allow_local_http: bool,
}

#[derive(Args)]
pub struct PublisherArgs {
    /// A publisher's domain
    #[arg(value_name = "DOMAIN")]
    pub domain: String,
    /// Print the result as JSON
    #[arg(
        long,
        long_help = "Print the result as JSON: the same object the language bindings' \
            publisher check gives, with the trust decisions under `discovery`."
    )]
    pub json: bool,
    /// Check time, RFC 3339 (default: now)
    #[arg(long, value_name = "TIME", allow_hyphen_values = true)]
    pub now: Option<String>,
}

#[derive(Args)]
pub struct ProbeArgs {
    /// The files to read
    #[arg(value_name = "FILE", required = true, num_args = 1..)]
    pub files: Vec<String>,
    /// Print the probes as a JSON array
    #[arg(
        long,
        long_help = "Print the probes as a JSON array, one object for each file read, with \
            the file's name in `file`. The rest of each object is what check compares with \
            the screens."
    )]
    pub json: bool,
}

#[derive(Args)]
pub struct LegibilityArgs {
    /// Nearest viewing distance, metres
    #[arg(long, value_name = "METRES", required = true, value_parser = parse_float, allow_hyphen_values = true)]
    pub near: f64,
    /// Farthest viewing distance, metres
    #[arg(long, value_name = "METRES", required = true, value_parser = parse_float, allow_hyphen_values = true)]
    pub far: f64,
    /// Text is legible at this multiple of its height (default 150)
    #[arg(long, value_name = "RATIO", value_parser = parse_float, allow_hyphen_values = true)]
    pub ratio: Option<f64>,
    /// The screen's width in millimetres, to give sizes in pixels
    #[arg(
        long = "screen-width-mm",
        value_name = "MM",
        value_parser = parse_float,
        allow_hyphen_values = true,
        requires = "screen_width_px"
    )]
    pub screen_width_mm: Option<f64>,
    /// The screen's width in pixels, with --screen-width-mm
    #[arg(
        long = "screen-width-px",
        value_name = "PX",
        allow_hyphen_values = true,
        requires = "screen_width_mm"
    )]
    pub screen_width_px: Option<i64>,
    /// Dwell time in seconds, to give a word limit
    #[arg(long, value_name = "SECONDS", value_parser = parse_float, allow_hyphen_values = true)]
    pub dwell: Option<f64>,
    /// Reading rate in words per second (default 2)
    #[arg(long, value_name = "WORDS", value_parser = parse_float, allow_hyphen_values = true)]
    pub rate: Option<f64>,
    /// Print the result as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct DwellArgs {
    /// The speed, km/h
    #[arg(long, value_name = "KMH", required = true, value_parser = parse_float, allow_hyphen_values = true)]
    pub speed: f64,
    /// Nearest viewing distance, metres
    #[arg(long, value_name = "METRES", required = true, value_parser = parse_float, allow_hyphen_values = true)]
    pub near: f64,
    /// Farthest viewing distance, metres
    #[arg(long, value_name = "METRES", required = true, value_parser = parse_float, allow_hyphen_values = true)]
    pub far: f64,
    /// Text is legible at this multiple of its height (default 150)
    #[arg(long, value_name = "RATIO", value_parser = parse_float, allow_hyphen_values = true)]
    pub ratio: Option<f64>,
    /// Reading rate in words per second (default 2)
    #[arg(long, value_name = "WORDS", value_parser = parse_float, allow_hyphen_values = true)]
    pub rate: Option<f64>,
    /// The screen's width in millimetres, to give sizes in pixels
    #[arg(
        long = "screen-width-mm",
        value_name = "MM",
        value_parser = parse_float,
        allow_hyphen_values = true,
        requires = "screen_width_px"
    )]
    pub screen_width_mm: Option<f64>,
    /// The screen's width in pixels, with --screen-width-mm
    #[arg(
        long = "screen-width-px",
        value_name = "PX",
        allow_hyphen_values = true,
        requires = "screen_width_mm"
    )]
    pub screen_width_px: Option<i64>,
    /// Show this built-in rule pack's limits at the speed, repeatable
    #[arg(
        long,
        value_name = "REF",
        action = ArgAction::Append,
        allow_hyphen_values = true,
        long_help = "Show the limits this built-in rule pack, name@version, sets at the \
            speed. Give it more than once for several packs; rule-packs lists them."
    )]
    pub pack: Vec<String>,
    /// Print the result as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct RulePacksArgs {
    /// A rule pack, as name@version (default: list them all)
    #[arg(value_name = "REF", allow_hyphen_values = true)]
    pub reference: Option<String>,
    /// Print as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct ConformanceArgs {
    /// The conformance directory
    #[arg(value_name = "DIR", default_value = "conformance/0.1")]
    pub dir: PathBuf,
    /// Level to test: core, layout or full
    #[arg(
        long,
        value_name = "LEVEL",
        default_value = "full",
        allow_hyphen_values = true
    )]
    pub level: String,
    /// External validator to test
    #[arg(
        long,
        value_name = "COMMAND",
        allow_hyphen_values = true,
        long_help = "External validator to test. It is run with each case directory as \
            its last argument and must print the reports as a JSON array."
    )]
    pub cmd: Option<String>,
    /// List passing cases too
    #[arg(short, long, alias = "v")]
    pub verbose: bool,
    /// Skip the cases for optional strictest requirements
    #[arg(
        long = "no-strictest",
        long_help = "Skip the cases for strictest requirements, which a validator may \
            offer but need not. Use it for a validator that doesn't."
    )]
    pub no_strictest: bool,
}

#[derive(Args)]
pub struct CompletionArgs {
    /// The shell to print completions for
    #[arg(value_enum, value_name = "SHELL")]
    pub shell: Shell,
}

/// The shells `completion` can write for.
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
    Powershell,
}

impl Shell {
    pub const ALL: [Shell; 4] = [Shell::Bash, Shell::Zsh, Shell::Fish, Shell::Powershell];

    fn to_clap(self) -> clap_complete::Shell {
        match self {
            Shell::Bash => clap_complete::Shell::Bash,
            Shell::Zsh => clap_complete::Shell::Zsh,
            Shell::Fish => clap_complete::Shell::Fish,
            Shell::Powershell => clap_complete::Shell::PowerShell,
        }
    }

    /// The file name completions for this shell are installed under.
    pub fn file_name(self) -> &'static str {
        match self {
            Shell::Bash => "opencrs.bash",
            Shell::Zsh => "_opencrs",
            Shell::Fish => "opencrs.fish",
            Shell::Powershell => "_opencrs.ps1",
        }
    }
}

/// Writes the completion script for `shell`.
pub fn write_completions(shell: Shell, out: &mut dyn std::io::Write) {
    clap_complete::generate(shell.to_clap(), &mut command(), "opencrs", out);
}

/// Go's `strconv.ParseFloat`, as the old parser reported it ("parse error" or
/// "value out of range"). Hexadecimal floats and underscores aren't accepted.
fn parse_float(s: &str) -> Result<f64, String> {
    let body = s.strip_prefix(['+', '-']).unwrap_or(s);
    let lower = body.to_ascii_lowercase();
    let named = matches!(lower.as_str(), "inf" | "infinity" | "nan");
    let plain = !body.is_empty()
        && body
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-'));
    if !named && !plain {
        return Err("parse error".into());
    }
    match s.parse::<f64>() {
        Ok(f) if f.is_infinite() && !named => Err("value out of range".into()),
        Ok(f) => Ok(f),
        Err(_) => Err("parse error".into()),
    }
}

fn help_arg() -> Arg {
    Arg::new("help")
        .short('h')
        .long("help")
        .action(ArgAction::Help)
        .help("Show help (--help shows more)")
        .global(false)
}

/// The whole command, for parsing, help, completions and the man page.
pub fn command() -> Command {
    let mut cmd = Cli::command().bin_name("opencrs").arg(help_arg()).arg(
        Arg::new("version")
            .short('V')
            .long("version")
            .action(ArgAction::Version)
            .help("Show the version"),
    );
    let names: Vec<String> = cmd
        .get_subcommands()
        .map(|s| s.get_name().to_string())
        .collect();
    for name in names {
        cmd = cmd.mut_subcommand(name, |s| s.disable_help_flag(true).arg(help_arg()));
    }
    cmd
}

/// Rewrites the old single-dash spellings to the forms clap understands.
///
/// - `-spec x`, `-no-files`, `-allow-local-http` become `--spec x` and so on
///   (any long flag may be written with one dash);
/// - a boolean flag may be given a value, `-json=false`, as Go's `flag`
///   package allows; `false` cancels earlier uses of the flag;
/// - `-help` is `--help`, and `help` or `help COMMAND` as the first argument
///   shows help.
///
/// A flag's value is passed through untouched, whatever it looks like, and
/// nothing after `--` is changed.
pub fn normalize(root: &Command, args: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(args.len());
    let mut cur = root;
    let mut i = 0;
    if args.first().map(String::as_str) == Some("help") {
        return match args.get(1) {
            Some(name) if root.find_subcommand(name).is_some() => {
                vec![name.clone(), "--help".into()]
            }
            _ => vec!["--help".into()],
        };
    }
    while i < args.len() {
        let a = &args[i];
        i += 1;
        if a == "--" {
            out.push(a.clone());
            out.extend(args[i..].iter().cloned());
            break;
        }
        let (dashes, body) = if let Some(r) = a.strip_prefix("--") {
            (2, r)
        } else if let Some(r) = a.strip_prefix('-') {
            (1, r)
        } else {
            if std::ptr::eq(cur, root)
                && let Some(sub) = root.find_subcommand(a)
            {
                cur = sub;
            }
            out.push(a.clone());
            continue;
        };
        let (name, value) = match body.split_once('=') {
            Some((n, v)) => (n, Some(v)),
            None => (body, None),
        };
        let single = name.chars().count() == 1;
        let arg = if name.is_empty() {
            None
        } else if single {
            let c = name.chars().next();
            cur.get_arguments().find(|x| x.get_short() == c)
        } else {
            cur.get_arguments().find(|x| {
                x.get_long() == Some(name)
                    || x.get_all_aliases().is_some_and(|al| al.contains(&name))
            })
        };
        let Some(arg) = arg else {
            out.push(a.clone());
            continue;
        };
        let canonical = match arg.get_long() {
            Some(l) => format!("--{l}"),
            None => format!("-{name}"),
        };
        let takes_value = arg.get_action().takes_values();
        if !takes_value {
            match value {
                None => out.push(if single { a.clone() } else { canonical }),
                Some(v) => match go_bool(v) {
                    Some(true) => out.push(canonical),
                    Some(false) => out.retain(|o| *o != canonical),
                    None => out.push(a.clone()),
                },
            }
            continue;
        }
        match value {
            Some(v) if single && dashes == 1 => out.push(format!("-{name}={v}")),
            Some(v) => out.push(format!("{canonical}={v}")),
            None => {
                out.push(if single && dashes == 1 {
                    a.clone()
                } else {
                    canonical
                });
                if let Some(next) = args.get(i) {
                    out.push(next.clone());
                    i += 1;
                }
            }
        }
    }
    out
}

/// Go's `strconv.ParseBool`.
fn go_bool(s: &str) -> Option<bool> {
    match s {
        "1" | "t" | "T" | "true" | "TRUE" | "True" => Some(true),
        "0" | "f" | "F" | "false" | "FALSE" | "False" => Some(false),
        _ => None,
    }
}
