# Zeron Nova branding

Zeron Nova uses an icy cyan faceted bust, a warm gold nova flare, and a midnight blue background. The artwork was edited from the previous Zeron app icon with the built-in ImageGen tool on 2026-10-15.

## Assets

| Purpose | Path |
| --- | --- |
| Original generated master | `dist/zeron-nova-source.png` |
| Shared full-bleed 1024 px icon | `dist/zeron.png` |
| macOS icon with mask, margin, and shadow | `dist/macos/icon-1024.png` |
| Desktop sign-in mark | `crates/ui/assets/brand/zeron-nova.png` |
| Android adaptive foreground | `apps/android/app/src/main/res/mipmap-xxxhdpi/ic_launcher_foreground.png` |
| iOS app icon | `apps/ios/Zeron/Assets.xcassets/AppIcon.appiconset/AppIcon1024.png` |
| iOS sign-in mark | `apps/ios/Zeron/Assets.xcassets/NovaLogo.imageset/nova-logo.png` |

Regenerate the platform sizes and framing on macOS with:

```sh
swift scripts/generate-nova-icons.swift
```

Regenerate the Windows icon, embedded in the executable and the installer, from `dist/zeron.png` with Pillow:

```sh
python3 -c "from PIL import Image; Image.open('dist/zeron.png').convert('RGBA').save('dist/windows/zeron.ico', sizes=[(s, s) for s in (16, 24, 32, 48, 64, 128, 256)])"
```

The iOS icon is opaque and full bleed; iOS supplies its own rounded mask. Android uses a navy adaptive background and a 10% foreground inset to keep the face and nova accent inside launcher masks. macOS packaging turns its framed PNG into the complete `.icns` size set.

The visible product name is **Zeron Nova** on desktop, Android and iOS. The macOS bundle identifier is `sh.zeron.app.nova` (`sh.zeron.app.nova.dev` for the development bundle); `scripts/package-macos.sh` and `scripts/run-macos-dev.sh` read the bundle name and identifier from `dist/macos/Info*.plist`. Mobile application identifiers, the iOS product and scheme names, mobile authentication URL schemes, Rust and Swift module names and CLI commands stay as upstream has them. The macOS updater archive keeps its internal `Zeron.app` path. Nova never checks for, offers or installs upstream releases (`zeron_update::UPSTREAM_RELEASES`); reinstall a local build with `scripts/install-nova-macos.sh`.

## Own identity

A Nova desktop build runs beside upstream Zeron on one machine. Every per-install name it would otherwise share comes from `crates/nova` (`zeron_nova`); upstream's defaults point there, and `crates/nova/tests/identity.rs` checks the files that ship outside Rust.

| Identity | Upstream | Nova |
| --- | --- | --- |
| Data directory (lock, logs, IPC token, adapters, cursor state, managed install) | `~/.zeron`, `%LOCALAPPDATA%\Zeron` | `~/.zeron-nova`, `%LOCALAPPDATA%\Zeron Nova` |
| Default worktrees and scratch chats | `~/.zeron/worktrees`, `~/.zeron/scratch` | `~/.zeron-nova/worktrees`, `~/.zeron-nova/scratch` |
| Engine IPC port | 27654 | 27664 |
| Development bundle IPC port (`scripts/run-macos-dev.sh`) | 49777 | 49778 |
| URL scheme (conversation and banner links) | `zeron://` | `zeron-nova://` |
| `zeron daemon` launchd label, systemd unit | `sh.zeron.app`, `zeron.service` | `sh.zeron.app.nova`, `zeron-nova.service` |
| Linux app id, `.desktop` entry, launcher binary link | `zeron` | `zeron-nova` |
| Windows installer `AppId`, install directory, URL class | `{AD5DEC34-…}`, `Programs\Zeron`, `Classes\zeron` | `{97089E9E-…}`, `Programs\Zeron Nova`, `Classes\zeron-nova` |

Nova never adopts upstream data: a new install starts empty, and the `.comet-native` migration stays upstream's. Every `ZERON_*` override still wins. Nova also opens an upstream `zeron://open/chat/…` link handed to it (`zeron <link>`). The sign-in loopback port (27641, `ZERON_CALLBACK_PORT`) and the edge stay shared: WorkOS accepts only the loopback redirect ports registered on the shared client (see `scripts/staging-app.sh`), so a Nova sign-in fails while a running upstream app that has opened its sign-in listener holds that port (`ZERON_CALLBACK_PORT` can move one of them only to another registered port).

## ImageGen prompt

Mode: built-in image editing, using the previous `dist/zeron.png` as the reference image; opaque background.

> Use case: precise-object-edit / logo-brand. Edit target: the attached current Zeron application logo. Create the final production master icon for the fork called Zeron Nova. Preserve its recognizable low-poly faceted classical marble bust looking three-quarter to the right, but give it a clearly distinct identity: luminous icy cyan and pearlescent blue stone, deep midnight blue hair and shadows, restrained warm gold rim light, and one small crisp four-point nova flare nestled above the left side of the hair. Deep navy full-bleed background with a subtle cyan-to-gold stellar halo behind the head. Simplify the busy background code texture substantially for small-size legibility. Refined sculptural, premium, strong readable facial silhouette and crisp triangular facets. Center the important head/face within the central 66% safe region so Android circle and iOS rounded-square cropping remain readable, shoulders may extend wider at the bottom. ONE square 1024x1024 icon, edge-to-edge fully opaque background; no rounded-square border, no outer padding, no drop shadow, no mockup, no lettering, no words, no watermark. This is a single master asset for consistent macOS, Android and iOS application icons, not a sheet of alternatives. Change color and the small nova accent while preserving the bust's identity and pose.
