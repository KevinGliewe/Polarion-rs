# Contributing

Bug reports and feature requests go to the [issues](https://github.com/KevinGliewe/Polarion-rs/issues); security
problems are reported privately ([SECURITY.md](.github/SECURITY.md)). The repository is English only.

## Where things are

- [CLAUDE.md](CLAUDE.md): the rules of the code base, for people and for coding agents alike: what is generated,
  which guard every write needs, which tests fail when a method or operation is not registered.
- [docs/](docs/README.md): the architecture, one page per mechanism.
- [docs/code-generation.md](docs/code-generation.md): the SOAP client is generated; change the generator, not its
  output.
- [docs/testing.md](docs/testing.md): the test layers, golden files, and the live suite against a real server.
- [docs/releasing.md](docs/releasing.md): how a tag becomes a release.

## Before you push

Run the command line under "Before pushing" in [CLAUDE.md](CLAUDE.md). It never contacts a Polarion server. CI
(`.github/workflows/ci.yml`) runs the same checks with warnings as errors, plus `cargo doc`, a build with the
minimum supported Rust version, and after a merge the tests on Windows and macOS.
