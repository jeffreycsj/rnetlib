#!/usr/bin/env python3
"""Exercise path dependencies from a separate Cargo workspace, without profile fixes.

Seed resolution from the SDK lockfile to avoid unrelated dependency upgrades. The
temporary consumer has its own workspace root: SDK profiles/patches cannot help it.
Both the normal test profile and explicitly checked release must pass.
"""

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile


def main():
    root = Path(__file__).resolve().parent.parent
    with tempfile.TemporaryDirectory(prefix="rnet-rust-consumer-") as directory:
        project = Path(directory)
        (project / "tests").mkdir()
        manifest = '\n'.join([
            '[package]', 'name = "rnet-rust-consumer"', 'version = "0.0.0"',
            'edition = "2021"', 'publish = false', '[workspace]',
            '[dependencies]', 'bytes = "1.10.1"',
            'rnet-core = { path = ' + json.dumps(str(root / 'crates/rnet-core')) + ' }',
            'rnet-transport = { path = ' + json.dumps(str(root / 'crates/rnet-transport')) + ' }',
            '',
        ])
        (project / "Cargo.toml").write_text(manifest, encoding="utf-8")
        shutil.copyfile(root / "Cargo.lock", project / "Cargo.lock")
        shutil.copyfile(root / "scripts/fixtures/rust-consumer.rs", project / "tests/boundaries.rs")
        shutil.copyfile(root / "crates/rnet-transport/tests/kcp_engine.rs", project / "tests/kcp_engine.rs")
        env = dict(os.environ, CARGO_BUILD_JOBS="1")
        env["CARGO_TARGET_DIR"] = str(root / "target/rust-consumer")
        for profile in ([], ["--release"]):
            checked_env = dict(env)
            if profile:
                checked_env["CARGO_PROFILE_RELEASE_OVERFLOW_CHECKS"] = "true"
            subprocess.run(
                ["cargo", "test", "--offline", *profile, "--", "--test-threads=1"],
                cwd=project, env=checked_env, check=True,
            )


if __name__ == "__main__":
    main()
