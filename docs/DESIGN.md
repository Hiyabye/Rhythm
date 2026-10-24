# Design — Quiet Intelligence

Rhythm is a personal command center, not a chatbot-first dashboard. The foundation should feel calm and useful even before sources are connected. Empty space communicates the absence of real information; it is not filled with fictional announcements.

## Korean-first interface

Main navigation is **오늘 · 받은 소식 · 탐색 · 보관함 · 설정**. Labels, empty states, settings feedback, and application-facing errors use natural Korean. Product identifiers and model names remain recognizable. Future features are described as planned rather than represented by buttons that pretend to work.

The daily page introduces a future briefing. The inbox, search, and saved-item views establish a consistent layout without claiming an archive exists. Settings is the functional center of v0.1.0, with sections for general preferences, extensions, AI, personalization, notifications, energy, privacy/data, and application information.

## Typography and layout

Pretendard Variable is bundled through the `pretendard` npm package and imported locally in `src/main.ts`; startup does not rely on a font CDN. The font stack falls back to Pretendard, native system fonts, and sans-serif. Pretendard uses the SIL Open Font License 1.1; see the package's `dist/LICENSE.txt`.

Hierarchy comes primarily from type size/weight, whitespace, and alignment. A persistent left sidebar gives navigation a stable location. Settings uses an internal section list and a restrained content column. At narrower widths the sidebar becomes icon navigation and settings controls reflow. The native window retains platform controls and a minimum usable size.

The UI styles live in `src/app.css`; reusable SVG symbols are in `src/lib/Icon.svelte`. Tailwind CSS is integrated, with custom semantic component classes defining the current visual system.

## Color

Brand anchors:

| Role | Color |
| --- | --- |
| Forest green | `#344C43` |
| Warm ivory | `#F4F1EA` |
| Dark foundation | `#202622` |
| Muted green reference | `#A1AA9C` |

The light UI refines ivory into a near-white main surface (`#FAF9F6`) and warmer sidebar (`#F0EEE7`). Dark mode uses the dark foundation, muted green text, and a lighter green accent. CSS variables define backgrounds, text, dividers, interactive surfaces, and errors; the dark theme changes the same tokens rather than introducing a separate visual language.

Color is restrained: selection, focus, and primary actions receive emphasis. Thin separators organize information instead of stacks of colorful cards. There are no purple gradients, glowing AI ornaments, or fabricated activity indicators.

## Components and interaction

- A four-bar mark anchors the sidebar and empty briefing state.
- Selected navigation combines background, text weight, and a small marker, not color alone.
- Theme is a segmented control; energy profiles are a compact option group.
- Extension switches represent persisted opt-in, while availability is separately described. An unavailable connector is never labeled operational.
- Settings save feedback and errors are announced with status/alert semantics.
- Disabled controls and explicit explanatory text distinguish future capabilities from operations.
- Text inputs have labels; icon-only buttons retain accessible names.
- Focus-visible outlines, a skip link, keyboard-operable native controls, and reduced-motion styles are included.

Theme selection takes effect immediately and is persisted through the Rust backend; a save failure restores the stored theme. System mode follows the OS color preference. Effects are limited to short interaction transitions. Accessibility and contrast still require interactive testing on the actual native webviews; CSS and semantic markup alone are not a completed accessibility audit.

## Beat branding

`assets/branding/beat.svg` is the original vector source. Four vertical rounded bars have different heights, creating a simple rhythm recognizable at small sizes. The initial app icon uses ivory bars on forest green with a rounded square background. Tauri-generated desktop icon formats live in `src-tauri/icons/` and should be retained in source control.

The in-app mark uses `currentColor` so it works in both themes. The initial branding is intentionally simple and may be refined later without changing navigation or application architecture. The original logo is covered by Rhythm's MIT license; font and other dependency licenses remain separate.
