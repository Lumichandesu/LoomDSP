//! The LoomDSP patch canvas (ADR-0010).
//!
//! An Iced widget that draws modules, ports, and cables with one custom wgpu
//! primitive. Shapes are instanced quads and cables are triangle strips, so
//! GPU cost follows the number of things drawn, not the number of pixels.
//! Text goes through Iced's text system. Zoomed out, where it is too small
//! to read, faint bars from the instanced buffer stand in for it.
//!
//! The application owns the data. It builds a [`Scene`], passes it to
//! [`PatchCanvas`], and applies the [`CanvasEvent`]s the canvas reports.

mod camera;
mod controls;
mod gpu;
mod hover;
mod scene;
mod theme;
mod tile_text;
mod widget;

pub use camera::{Camera, MAX_ZOOM, MIN_ZOOM};
pub use controls::{ModulationMark, PanelValues, WidgetValue};
pub use scene::{
    CableView, GRID, Hit, HoverText, LABEL_COLUMNS, ModulationView, ModuleView, PanelView, PortRef, PortShape,
    PortView, Scene, TILE_WIDTH, WidgetView, WidgetViewKind, fit_label, snap, tile_size,
};
pub use theme::Palette;
pub use widget::{CanvasEvent, CanvasView, LearnView, PatchCanvas, Still, StillDrag};
