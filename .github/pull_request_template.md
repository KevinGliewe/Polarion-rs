<!-- What does this change, and why? Link the issue if there is one. -->

## Checklist

- [ ] The checks in [CONTRIBUTING.md](https://github.com/KevinGliewe/Polarion-rs/blob/main/CONTRIBUTING.md#before-you-push) pass.
- [ ] Generated code was regenerated (`cargo xtask codegen`), not edited by hand.
- [ ] A method that modifies data calls a write guard first, and a new SOAP write operation is classified in
      `crates/polarion/tests/write_targets.rs`.
- [ ] A new public library method has a command, and the `docs/` page of a changed mechanism is updated.
