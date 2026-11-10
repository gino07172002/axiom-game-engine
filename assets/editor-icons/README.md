# Axiom editor icons

106 original monochrome SVG icons created for Axiom. All contours were drawn for
this project; no third-party sources, icon sets, fonts, or glyphs were used.

- 24 × 24 viewBox and intrinsic size; artwork including strokes stays within 2 px padding.
- 1.75 px strokes, round caps and joins, open contours, consistent optical scale.
- `stroke="currentColor"`, `fill="none"`; only small solid details use `fill="currentColor"`.
- Pure paths and basic shapes. No text, embedded images, scripts, or external references.
- Inline SVGs inherit CSS `color`. When loaded through `<img>`, the parent's color does
  not inherit into the file; inline SVGs for theme tinting (as in `preview.html`).
- `preview.html` is an offline, static contact sheet: 16, 24, and 48 px in white
  and accent `#ff4d6d`, on `#15171c`. `icons.json` records filenames and tags.

## Full list

**Tools:** select, move, rotate, scale, rect-tool, snap, grid, list, pivot, center, global, local, frame, fullscreen, screenshot.

**Playback:** play, pause, stop, step, time-scale, record.

**Edit:** undo, redo, save, add, duplicate, delete, rename, copy, paste, reset, more, search, filter, close, check, chevron-down, chevron-right, chevron-up, drag-handle, external-link, link, unlink.

**Visibility:** eye, eye-off, lock, unlock.

**Panels:** hierarchy, inspector, assets, console, history, proposals, problems, scenarios, graphs, game, scene, world, layout, settings, agent, build, help.

**Objects / components:** entity, group, folder, folder-open, file, mesh, material, texture, animation, skeleton, audio, audio-listener, light-point, light-spot, light-directional, light-area, camera, camera-follow, trigger, path, nav, interact, tween, fsm, behavior, ui-document, prefab, collider, vfx, script, tag, layers, input, localization, event.

**Status:** chip, info, warning, error, success, loading, dot.

## Small-size notes

`local`, `mesh`, `prefab`, `camera-follow`, `skeleton`, and `localization` use
spaced contours and simplified details for 16 px. Connected edges and intentional
junctions meet; separate strokes stay about 2.5 units apart and lines are at least
about 3 units long. `global` is deliberately simpler than `world`.
`record` and `dot` use different solid-disc sizes.

## License

Same license as the Axiom repository. These assets do not introduce a separate license.
