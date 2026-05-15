# Security Policy

If you discover a security vulnerability in texttv, please report it privately
via [GitHub Security Advisories][advisory] rather than opening a public issue
or pull request. I'll acknowledge receipt within 7 days and coordinate a fix
and disclosure timeline from there.

The main attack surface to be aware of:

- Untrusted HTTP responses from `svt.se` and `api.texttv.nu` flow through
  `scraper` (HTML parser) and `image` (GIF decoder); both parse hostile bytes.
- The on-disk mosaic cache under the platform cache directory holds 1-byte
  files keyed by GIF content-hash.

[advisory]: https://github.com/dfallman/texttv/security/advisories/new
