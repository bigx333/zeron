# Zeron Nova branding

Zeron Nova uses an icy cyan faceted bust, a warm gold nova flare, and a midnight blue background. The artwork was edited from the previous Zeron app icon with the built-in ImageGen tool on 2026-10-15.

## Assets

| Purpose | Path |
| --- | --- |
| Original generated master | `dist/zeron-nova-source.png` |
| Purple comparison variant | `dist/zeron-nova-purple-source.png` |
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

The iOS icon is opaque and full bleed; iOS supplies its own rounded mask. Android uses a navy adaptive background and a 10% foreground inset to keep the face and nova accent inside launcher masks. macOS packaging turns its framed PNG into the complete `.icns` size set.

The visible product name is **Zeron Nova** across desktop, Android, and iOS. The macOS bundle identity is `sh.zeron.app.nova`. Mobile application identifiers, authentication URL schemes, Rust and Swift module names, CLI commands, and existing data locations remain compatible with previous builds. The iOS app product is `Zeron Nova.app`, with the `Zeron` scheme and executable retained for tooling. The macOS updater archive retains its legacy internal `Zeron.app` path for compatibility; the installer presents `Zeron Nova.app`. Nova never checks for, offers or installs upstream releases (`zeron_update::UPSTREAM_RELEASES`); reinstall a local build with `scripts/install-nova-macos.sh`.

## ImageGen prompt

Mode: built-in image editing, using the previous `dist/zeron.png` as the reference image; opaque background.

> Use case: precise-object-edit / logo-brand. Edit target: the attached current Zeron application logo. Create the final production master icon for the fork called Zeron Nova. Preserve its recognizable low-poly faceted classical marble bust looking three-quarter to the right, but give it a clearly distinct identity: luminous icy cyan and pearlescent blue stone, deep midnight blue hair and shadows, restrained warm gold rim light, and one small crisp four-point nova flare nestled above the left side of the hair. Deep navy full-bleed background with a subtle cyan-to-gold stellar halo behind the head. Simplify the busy background code texture substantially for small-size legibility. Refined sculptural, premium, strong readable facial silhouette and crisp triangular facets. Center the important head/face within the central 66% safe region so Android circle and iOS rounded-square cropping remain readable, shoulders may extend wider at the bottom. ONE square 1024x1024 icon, edge-to-edge fully opaque background; no rounded-square border, no outer padding, no drop shadow, no mockup, no lettering, no words, no watermark. This is a single master asset for consistent macOS, Android and iOS application icons, not a sheet of alternatives. Change color and the small nova accent while preserving the bust's identity and pose.

Purple comparison prompt (built-in edit of the generated cyan master):

> Create ONE purple colorway of this exact new Zeron Nova application icon for visual comparison. Preserve the composition, faceted classical bust, pose and silhouette, crisp triangular facets, precise four-point gold nova star above the left hair, restrained gold rim light, and full-bleed square format. Change the icy cyan/blue marble into luminous amethyst, violet, and pearlescent lavender marble, with rich deep-plum and midnight-purple hair/shadows. Shift the navy stellar background into very dark aubergine and violet with a subtle purple-to-gold stellar halo. Keep the face readable and luminous, avoid pink/magenta dominance, keep the warm gold accent restrained, premium sculptural appearance. Same scale and placement, opaque edge-to-edge background. No text, no lettering, no border, no outer padding, no rounded mask, no mockup, no sheet. This is a purple alternative to the attached new cyan-and-gold Nova icon, not a return to the original Zeron icon.
