# Starter files

The smallest valid version of each OpenCRS document, at the Core conformance level, to copy and fill in. Replace `your-domain.example` with your own domain.

| File | Who writes it | Where it goes |
| --- | --- | --- |
| `opencrs.json` | The publisher (media owner or landlord) | `https://<your domain>/.well-known/opencrs.json` |
| `screens.json` | The publisher | The URL the index lists |
| `creative.json` | The agency or creative tool | Delivered with the creative files |

Each file names its JSON Schema in `$schema`, so editors such as VS Code check fields and suggest them as you type. For safe areas, viewing distances, rules and the rest, see the complete example in [`../harbourline/`](../harbourline/) and the specification.
