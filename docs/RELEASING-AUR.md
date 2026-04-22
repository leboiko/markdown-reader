# Publishing to the AUR (Arch User Repository)

`markdown-reader-bin` ships as a binary AUR package so Arch Linux users can
install with `yay -S markdown-reader-bin` (or any AUR helper). Once the
package exists on the AUR, the release workflow auto-pushes new versions on
every `v*` tag — but the **first** publish has to be done by hand because
AUR creates the repo on first push and we need the maintainer's SSH key
registered with their AUR account.

## One-time setup (do this once per maintainer)

1. **Create an AUR account** at <https://aur.archlinux.org/register>. The
   username doesn't have to match your GitHub handle.

2. **Register your SSH public key** at
   <https://aur.archlinux.org/account/> → "SSH Public Key" field. Paste
   the contents of `~/.ssh/id_ed25519.pub` (or whichever key you'll use
   from your laptop AND from CI).

3. **Reserve the package name** by cloning the to-be-created repo:
   ```sh
   git clone ssh://aur@aur.archlinux.org/markdown-reader-bin.git
   cd markdown-reader-bin
   ```
   The clone will succeed with an empty repo — that's expected. AUR
   creates the repo lazily on first push.

4. **Render the PKGBUILD + .SRCINFO** for the latest release:
   ```sh
   gh release download vX.Y.Z -R leboiko/markdown-reader -p SHA256SUMS \
     -D /tmp
   /path/to/markdown-reader/scripts/render-aur-pkgbuild.sh \
     X.Y.Z /tmp/SHA256SUMS .
   ```
   This writes `PKGBUILD` and `.SRCINFO` into the current directory.

5. **Commit and push:**
   ```sh
   git add PKGBUILD .SRCINFO
   git commit -m "markdown-reader X.Y.Z"
   git push
   ```

   First push registers the package on AUR. Visit
   <https://aur.archlinux.org/packages/markdown-reader-bin> to confirm it
   appears.

## Wiring the GitHub Action for auto-publish

After the one-time setup, configure the release workflow to auto-publish
on every `v*` tag:

1. **Generate a CI-only SSH key** (don't reuse your personal one):
   ```sh
   ssh-keygen -t ed25519 -f /tmp/aur-ci-key -C "github-actions ci"
   ```
   This produces `/tmp/aur-ci-key` (private) and `/tmp/aur-ci-key.pub`
   (public).

2. **Add the public key to your AUR account** at
   <https://aur.archlinux.org/account/>. AUR allows multiple SSH keys
   per account — keep your personal key + add the CI key.

3. **Add the private key as a GitHub secret** named `AUR_SSH_KEY`:
   ```sh
   gh secret set AUR_SSH_KEY \
     --repo leboiko/markdown-reader \
     < /tmp/aur-ci-key
   ```
   (Or via the web UI: Repo → Settings → Secrets and variables →
   Actions → New repository secret.)

4. **Delete the local copy** of the CI private key:
   ```sh
   rm /tmp/aur-ci-key /tmp/aur-ci-key.pub
   ```

After the secret is set, the next `v*` tag push will run the
`publish-aur` job in `.github/workflows/release.yml` — it clones the
AUR repo, renders the PKGBUILD + .SRCINFO, and pushes a commit named
`markdown-reader X.Y.Z`. Until the secret is set, the job runs but
no-ops cleanly (logs the skip reason and exits successfully).

## Publishing a fresh release manually (no GitHub Action)

If you ever want to skip CI and publish a release by hand:

```sh
cd ~/path/to/aur-checkout/markdown-reader-bin
gh release download vX.Y.Z -R leboiko/markdown-reader -p SHA256SUMS \
  -D /tmp
~/path/to/markdown-reader/scripts/render-aur-pkgbuild.sh \
  X.Y.Z /tmp/SHA256SUMS .
git add PKGBUILD .SRCINFO
git commit -m "markdown-reader X.Y.Z"
git push
```

That's the same flow the CI job runs.

## What ships in `markdown-reader-bin`

The AUR package is a thin wrapper around the GitHub Release tarballs:

- Architectures: `x86_64`, `aarch64` (both Linux GNU)
- Sources: `markdown-reader-X.Y.Z-{arch}-unknown-linux-gnu.tar.gz` from
  `https://github.com/leboiko/markdown-reader/releases/download/vX.Y.Z/`
- Installed file: `/usr/bin/markdown-reader`
- License: MIT, installed at `/usr/share/licenses/markdown-reader-bin/LICENSE`
- `provides=('markdown-reader')` + `conflicts=('markdown-reader')` so the
  -bin package coexists with a future source-build `markdown-reader`
  AUR package.
- `options=('!strip')` because the GitHub Release binaries are already
  stripped.
