# Metal view preparation

`prepare` is used only with the plugins' own baseview rendering NSViews. The
borrowed raw-window-handle contract guarantees a live NSView for the call. A
MainThreadMarker check precedes its one raw-pointer dereference. The pointer is
not retained or returned; all later operations use safe objc2 APIs. AppKit retains
the installed system CAMetalLayer. wgpu separately retains it when creating the
surface, so the wrapper transfers no borrowed pointer to the renderer.

The layer is a system class, with no project-defined Objective-C subclass or
observer callbacks. raw-window-metal's public CAMetalLayer reuse path therefore
does not register its `RawWindowMetalLayer` observer. Independent copies of a
plugin, or a host and plugin with separate statically linked renderer copies,
can use this path without competing to register that observer class.

Replacing a root layer changes drawing semantics. Consequently this API is for
our owned plugin rendering views, not a DAW's supplied parent NSView. The plugin
continues drawing through its existing display-synchronized callback/baseview
frame timer. Bounds and content scale are updated on resize/scale events;
surface recovery reuses the same layer. No dependency source is changed.

`cargo run -p macos-metal-view --bin metal-view-check` verifies creation and
reuse on two native views at different scales without registering the observer
class. It runs on the real main thread and needs no GPU adapter. The binary's
raw handle adapter borrows a retained NSView and cannot outlive it. This check
does not replace opening and resizing plugin editors in a desktop host.
