## Summary

<!-- Describe what this PR changes and why. -->

## Checklist

- [ ] `cargo fmt --all -- --check` passes
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings` passes
- [ ] `cargo test --workspace` passes
- [ ] `docs/COMPATIBILITY_REPORT.md` updated (if the compatibility corpus changed)
- [ ] Zero machine-local path leaks (no local home or temp directory paths in code, tests, or docs)
