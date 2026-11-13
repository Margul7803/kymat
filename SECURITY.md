# Security Policy

## Supported Versions

Only the latest release on the `main` branch is actively supported with security updates.

## Reporting a Vulnerability

If you discover a security issue (for example in external network requests, local file/database parsing, or shell scripts), please **do not open a public issue** with exploit details.

Instead, please report it privately via GitHub Security Advisories on [Margul7803/kymat](https://github.com/Margul7803/kymat/security/advisories/new).

## Network & Local Data Scope

- Kymat connects to `https://lrclib.net` over TLS (`rustls`) to fetch synchronized lyrics when not cached locally.
- Kymat reads local media metadata via D-Bus MPRIS, captures audio monitor output via PipeWire/PulseAudio, and optionally reads `~/.cache/sonora/cache.sqlite` in read-only mode.
