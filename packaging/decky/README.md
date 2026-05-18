# Steam Deck Decky Package

This directory contains the Decky Loader plugin used by the Steam Deck bootstrap
and update flow.

## Build Release Assets

Run from the repository root on Linux:

```bash
RDP_VERSION=0.1.0 \
RDP_CHANNEL=dev \
scripts/build-steamdeck-release.sh
```

The script writes these files to `dist/steamdeck/`:

- `rabbit-digger-pro-x86_64-unknown-linux-gnu`
- `rabbit-digger-pro-decky.zip`
- `steamdeck-update-manifest.json`

When a GitHub Release is published, the `Steam Deck Release Assets` workflow
builds these assets automatically and uploads them to that release. The Decky
plugin checks the release manifest at `releases/latest/download`, so publishing a
new stable release is what makes the Quick Access Menu update button pick up a
new version. The workflow can also be run manually with a `release_tag` to upload
assets to an existing release.

## Install On Steam Deck

The bootstrap installer consumes the manifest and installs both the host helper
and the Decky plugin. If Decky Loader is missing, the installer tries Decky's
official release installer first. If GitHub is unreachable from the Steam Deck,
the Rabbit Digger Pro helper and plugin files are still installed; install Decky
Loader later and it can pick up the staged plugin.

```bash
RDP_MANIFEST_URL=https://github.com/spacemeowx2/rabbit-digger-pro/releases/latest/download/steamdeck-update-manifest.json \
scripts/install-steamdeck.sh
```

After bootstrap, normal updates should be done from the Decky Quick Access Menu.

Set `RDP_SKIP_DECKY_INSTALL=1` to skip Decky Loader auto-install. Set
`RDP_REQUIRE_DECKY=1` if the bootstrap should fail when Decky Loader is missing.

## Uninstall From Steam Deck

The same bootstrap script can remove Rabbit Digger Pro's user service, helper
binary, Decky plugin, token, and update config:

```bash
scripts/install-steamdeck.sh uninstall
```

Decky Loader itself is left installed because it may be shared by other plugins.
