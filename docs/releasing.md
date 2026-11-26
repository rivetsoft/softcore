# Releasing

How Softcore gets from its changelog to a download on GitHub and on the website. It is the
Rivet apps' release flow (rivetsoft/apps, `docs/releasing.md`) for a repository of one program,
with no AppImage: a compositor is started by the login screen, not run from a file.

## What a release is

| Thing | Softcore 0.1.0 |
| --- | --- |
| Tag | `softcore-v0.1.0` |
| GitHub release | "Softcore 0.1.0", a pre-release until `STABLE` is set in `utilities/softcore-dev` |
| Download | `softcore-0.1.0-x86_64-linux.tar.gz`, and its `.sha256` |
| Source | GitHub's "Source code" archives at the tag |
| Notes | the README's first picture, the version's section of `CHANGELOG.md`, then how to install |

The download holds the program, `install.sh`, the README, the changelog, the manual and the
licenses. Its `install.sh` is the one in the repository: beside a built program it installs
that one for everyone (`softcore install --system`, with sudo); in the source code it builds
first.

The program is built on Ubuntu 24.04, so it needs glibc 2.39 or newer, and loads the system's
libseat, libinput, libudev, libgbm, libdrm, EGL, libxkbcommon, Wayland and PipeWire, whose
library names haven't changed in years. libdisplay-info is the exception: distributions ship
0.1, 0.2 or 0.3 under different names, so the release builds 0.3.0 itself and links it into the
program.

Rivet Welcome doesn't list or install Softcore.

## Releasing a version

1. **As you work**, add notes for people under `## Unreleased` in `CHANGELOG.md`.
2. **Bump in a pull request**: the version in `Cargo.toml` (and `cargo check` for
   `Cargo.lock`), and `## Unreleased` becomes `## X.Y.Z`. Before 1.0, a minor version for
   something new a person can do, a patch for fixes. Merge it once CI passes.
3. **Ship.** On `main`, up to date with GitHub:

   ```sh
   utilities/softcore-dev ship --dry-run   # says which tag it would push
   utilities/softcore-dev ship             # tags main and pushes the tag
   ```

4. **GitHub builds it.** The tag starts [the Release workflow](../.github/workflows/release.yml)
   on Ubuntu 24.04. It checks the tag matches `Cargo.toml` and is on `main`, runs
   `utilities/softcore-dev release`, then unpacks the download, runs its `install.sh`, asks the
   installed program its version and uninstalls it. Then it makes a **draft** release with the
   download and the notes. Follow it with `gh run list -R rivetsoft/softcore --workflow release.yml`.
5. **Look at the draft** on the releases page: the notes, the two files.
6. **Go live**: `utilities/softcore-dev live` publishes the draft and starts the website's
   rebuild, which shows the version and the download on Softcore's page.

### A rehearsal

Actions → Release → Run workflow builds the download from the branch chosen and keeps it with
the run, with no tag and no release. A pull request that changes the workflow, the script or
`install.sh` does the same. On a computer, `utilities/softcore-dev release` builds the same
files in `dist/`; it needs meson, ninja and hwdata for libdisplay-info, and stops if the
program needs a newer glibc than 2.39, as anything built on Fedora 41 or later does.

### When something goes wrong

- **The build failed**: fix it on `main` in a pull request, delete the tag
  (`git push origin :refs/tags/softcore-v0.1.0 && git tag -d softcore-v0.1.0`) and ship again.
  A tag nobody has published yet can move; a published one never does.
- **The draft is wrong**: delete the draft on GitHub, fix, delete the tag and ship again.
- **Published and wrong**: release the next patch version.
