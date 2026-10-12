//! shadcn/ui-style components for egui.
//!
//! Call [`Theme::install`] once (for example in your app's `new`), then use
//! the widgets like any egui widget:
//!
//! ```ignore
//! use widgets::*;
//! Theme::light().install(ctx);
//! if ui.add(Button::outline("Cancel")).clicked() { /* ... */ }
//! ui.add(Input::new(&mut email).placeholder("Email"));
//! ```

mod basic;
mod button;
mod data_table;
mod form;
mod layout;
mod overlay;
mod theme;
mod tree;

pub use basic::{h1, h2, h3, kbd, label, muted, separator, Avatar, Badge, BadgeVariant, Progress, Skeleton};
pub use button::{Button, ButtonSize, ButtonVariant};
pub use data_table::{Column, DataTable, DataTableResponse};
pub use form::{radio_group, toggle_group, Checkbox, Input, RadioItem, Slider, Switch, Textarea, Toggle};
pub use layout::{tabs, Accordion, Alert, Card, Table};
pub use overlay::{
    alert_dialog, confirm_dialog, dropdown_menu, footer, menu_checkbox, menu_item, menu_label,
    menu_separator, popover, toast, toast_error, Dialog, DialogResponse, Select, Toaster, TooltipExt,
};
pub use theme::{mix, paint_focus_ring, Theme};
pub use tree::{Tree, TreeSelection, TreeUi};

pub mod demo;
