# Workspace limitations

Floating panels, toolbars and the command line stay inside the main application window. Native floating windows on a second monitor are not supported. On a PC, when a second monitor is disconnected, the operating system moves the main window back to an available screen; floating controls follow that window and are clamped to its work area. This monitor-disconnection behavior has not been tested on physical hardware (UX07 partial).

Status-bar item visibility belongs to the workspace profile. Hiding a drafting toggle does not change its system variable; those settings continue to persist through `PROFILE_VARS`. The schema IDs `lineweight`, `transparency`, `quickprops`, `annotation`, `workspace`, and `settings` address the existing lineweight, transparency, Quick Properties, annotation visibility/scale, workspace picker, and Drafting Settings controls. `layouts` hides the layout tabs and their add/list buttons; `coordinates` hides the coordinate readout. Unknown status-bar IDs have no control to render: they are retained in the profile and ignored without a placeholder.

SIMPLE has neither a ribbon nor Quick Access buttons. Its default command line floats at the bottom centre with the existing 48% width rule (360–760 points, bounded by the canvas). Top and Bottom command lines reserve their own row, sized by `height_ratio` and constrained to leave room for the drawing. Interface scale is 0.75–2.0; the Options slider uses steps of 0.05, and egui keyboard zoom is saved through the normal workspace debounce.

The in-window menu bar scrolls horizontally when its menus and Quick Access exceed the available width. When zoom or a shorter window leaves insufficient height for the drawing, the ribbon renders as tabs only. The saved expanded/collapsed preference is preserved, so groups reappear when the window has enough room.


Workspace synchronization currently uses an in-memory ProfileRemote test double. No HTTP
backend, authenticated tenant identity, tenant isolation (UX10), server-side RBAC/ABAC or policy
locks, Ed25519-signed company publication, or publish/migration/restore audit log is implemented.
The `/api/v1/cad-workspaces` prefix in openapi.yaml is a proposal awaiting approval to avoid
Drupal's existing `/api/v1/workspace` NAS endpoint. HTTP uses RFC 6902 JSON Patch; adapting the client's
custom keyed-array merge patches is future work.

Import/export screening bounds hostile payloads and rejects executable hooks, secret-like keys
and sensitive paths in Import mode. It is not general secret-content detection or a LISP sandbox;
unknown inert text can be preserved without execution. Local storage and policy files are not
authorization boundaries or encrypted credential stores. UX12 end-to-end loaded-files/command-history
evidence remains outstanding until the profile_import_never_runs_lisp_ux12 test lands with the LISP panel.
