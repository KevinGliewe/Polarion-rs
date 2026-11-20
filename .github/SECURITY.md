# Security policy

## Supported versions

Fixes go into the latest 0.x release. Older versions are not patched.

## Reporting a vulnerability

Report it privately: [Report a vulnerability](https://github.com/KevinGliewe/Polarion-rs/security/advisories/new)
(Security > Advisories). Do not open a public issue for it.

Of particular interest:

- a write that gets past the write guard or restricted mode (`restrict_writes`, `POLARION_WRITE_PROJECTS`,
  `POLARION_WRITE_TARGETS`), or past a read-only client;
- a way around the token check of the MCP server over HTTP;
- credentials or tokens that end up in output, logs, error messages or files;
- Markdown or rich text conversion that lets unsafe HTML through.

Leave out host names, project IDs, user names and tokens of your server, or replace them.

This project has one maintainer; reports are answered as soon as possible, without a guaranteed time.
