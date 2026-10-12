use egui::{
    Color32, Context, CornerRadius, FontId, Frame, Id, Margin, Shadow, Stroke,
    TextStyle,
};

/// Design tokens in the spirit of shadcn/ui's CSS variables (neutral base color).
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub dark: bool,
    pub background: Color32,
    pub foreground: Color32,
    pub card: Color32,
    pub card_foreground: Color32,
    pub popover: Color32,
    pub popover_foreground: Color32,
    pub primary: Color32,
    pub primary_foreground: Color32,
    pub secondary: Color32,
    pub secondary_foreground: Color32,
    pub muted: Color32,
    pub muted_foreground: Color32,
    pub accent: Color32,
    pub accent_foreground: Color32,
    pub destructive: Color32,
    pub destructive_foreground: Color32,
    pub border: Color32,
    pub input: Color32,
    pub ring: Color32,
    /// Base radius, like `--radius` (0.625rem = 10px).
    pub radius: f32,
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

impl Theme {
    pub fn light() -> Self {
        Self {
            dark: false,
            background: hex(0xffffff),
            foreground: hex(0x0a0a0a),
            card: hex(0xffffff),
            card_foreground: hex(0x0a0a0a),
            popover: hex(0xffffff),
            popover_foreground: hex(0x0a0a0a),
            primary: hex(0x171717),
            primary_foreground: hex(0xfafafa),
            secondary: hex(0xf5f5f5),
            secondary_foreground: hex(0x171717),
            muted: hex(0xf5f5f5),
            muted_foreground: hex(0x737373),
            accent: hex(0xf5f5f5),
            accent_foreground: hex(0x171717),
            destructive: hex(0xe7000b),
            destructive_foreground: hex(0xffffff),
            border: hex(0xe5e5e5),
            input: hex(0xe5e5e5),
            ring: hex(0xa1a1a1),
            radius: 10.0,
        }
    }

    pub fn dark() -> Self {
        Self {
            dark: true,
            background: hex(0x0a0a0a),
            foreground: hex(0xfafafa),
            card: hex(0x171717),
            card_foreground: hex(0xfafafa),
            popover: hex(0x171717),
            popover_foreground: hex(0xfafafa),
            primary: hex(0xe5e5e5),
            primary_foreground: hex(0x171717),
            secondary: hex(0x262626),
            secondary_foreground: hex(0xfafafa),
            muted: hex(0x262626),
            muted_foreground: hex(0xa1a1a1),
            accent: hex(0x262626),
            accent_foreground: hex(0xfafafa),
            destructive: hex(0xff6467),
            destructive_foreground: hex(0xffffff),
            border: hex(0x282828),
            input: hex(0x343434),
            ring: hex(0x737373),
            radius: 10.0,
        }
    }

    fn id() -> Id {
        Id::new("widgets::theme")
    }

    /// The installed theme, or [`Theme::light`] if none was installed.
    pub fn get(ctx: &Context) -> Self {
        Self::installed(ctx).unwrap_or_else(Self::light)
    }

    /// The theme passed to [`Theme::install`], if any.
    pub fn installed(ctx: &Context) -> Option<Self> {
        ctx.data(|d| d.get_temp::<Theme>(Self::id()))
    }

    /// Stores the theme and restyles egui's own widgets to match it.
    pub fn install(self, ctx: &Context) {
        let t = self.clone();
        ctx.all_styles_mut(move |style| t.apply_to_style(style));
        ctx.set_theme(if self.dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        });
        ctx.data_mut(|d| d.insert_temp(Self::id(), self));
    }

    pub fn radius_sm(&self) -> CornerRadius {
        CornerRadius::same((self.radius - 4.0).max(0.0) as u8)
    }
    pub fn radius_md(&self) -> CornerRadius {
        CornerRadius::same((self.radius - 2.0).max(0.0) as u8)
    }
    pub fn radius_lg(&self) -> CornerRadius {
        CornerRadius::same(self.radius as u8)
    }
    pub fn radius_xl(&self) -> CornerRadius {
        CornerRadius::same((self.radius + 4.0) as u8)
    }

    /// The soft outer focus ring (`ring-ring/50`).
    pub fn ring_soft(&self) -> Color32 {
        self.ring.gamma_multiply(0.5)
    }

    pub fn shadow_sm(&self) -> Shadow {
        Shadow {
            offset: [0, 1],
            blur: 3,
            spread: 0,
            color: Color32::from_black_alpha(if self.dark { 60 } else { 18 }),
        }
    }

    pub fn shadow_lg(&self) -> Shadow {
        Shadow {
            offset: [0, 8],
            blur: 24,
            spread: 0,
            color: Color32::from_black_alpha(if self.dark { 120 } else { 40 }),
        }
    }

    /// Frame used by popovers, selects and dropdown menus.
    pub fn popover_frame(&self) -> Frame {
        Frame::new()
            .fill(self.popover)
            .stroke(Stroke::new(1.0, self.border))
            .corner_radius(self.radius_md())
            .inner_margin(Margin::same(4))
            .shadow(self.shadow_lg())
    }

    pub fn apply_to_style(&self, style: &mut egui::Style) {
        use egui::FontFamily::{Monospace, Proportional};
        style.text_styles = [
            (TextStyle::Small, FontId::new(12.0, Proportional)),
            (TextStyle::Body, FontId::new(14.0, Proportional)),
            (TextStyle::Button, FontId::new(14.0, Proportional)),
            (TextStyle::Heading, FontId::new(22.0, Proportional)),
            (TextStyle::Monospace, FontId::new(13.0, Monospace)),
        ]
        .into();

        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(16.0, 8.0);
        style.spacing.interact_size.y = 36.0;
        style.spacing.window_margin = Margin::same(24);
        style.spacing.menu_margin = Margin::same(4);

        let v = &mut style.visuals;
        v.dark_mode = self.dark;
        v.panel_fill = self.background;
        v.window_fill = self.popover;
        v.window_stroke = Stroke::new(1.0, self.border);
        v.window_corner_radius = self.radius_lg();
        v.window_shadow = self.shadow_lg();
        v.popup_shadow = self.shadow_lg();
        v.menu_corner_radius = self.radius_md();
        v.extreme_bg_color = self.background;
        v.text_edit_bg_color = Some(Color32::TRANSPARENT);
        v.faint_bg_color = self.muted;
        v.code_bg_color = self.muted;
        v.hyperlink_color = self.foreground;
        v.error_fg_color = self.destructive;
        v.selection.bg_fill = self.ring.gamma_multiply(0.35);
        v.selection.stroke = Stroke::new(1.0, self.ring);

        let w = &mut v.widgets;
        w.noninteractive.bg_fill = self.background;
        w.noninteractive.weak_bg_fill = self.background;
        w.noninteractive.bg_stroke = Stroke::new(1.0, self.border);
        w.noninteractive.fg_stroke = Stroke::new(1.0, self.foreground);
        w.noninteractive.corner_radius = self.radius_md();

        w.inactive.bg_fill = self.input;
        w.inactive.weak_bg_fill = self.background;
        w.inactive.bg_stroke = Stroke::new(1.0, self.input);
        w.inactive.fg_stroke = Stroke::new(1.0, self.foreground);
        w.inactive.corner_radius = self.radius_md();

        w.hovered.bg_fill = self.ring;
        w.hovered.weak_bg_fill = self.accent;
        w.hovered.bg_stroke = Stroke::new(1.0, self.input);
        w.hovered.fg_stroke = Stroke::new(1.0, self.accent_foreground);
        w.hovered.corner_radius = self.radius_md();
        w.hovered.expansion = 0.0;

        w.active.bg_fill = self.muted_foreground;
        w.active.weak_bg_fill = self.accent;
        w.active.bg_stroke = Stroke::new(1.0, self.ring);
        w.active.fg_stroke = Stroke::new(1.0, self.accent_foreground);
        w.active.corner_radius = self.radius_md();
        w.active.expansion = 0.0;

        w.open = w.active;
    }
}

/// Linear mix of two colors: `t = 0` gives `a`, `t = 1` gives `b`.
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(
        l(a.r(), b.r()),
        l(a.g(), b.g()),
        l(a.b(), b.b()),
        l(a.a(), b.a()),
    )
}

/// Paints the two-layer focus ring shadcn uses (`ring-[3px] ring-ring/50`).
pub fn paint_focus_ring(painter: &egui::Painter, rect: egui::Rect, radius: CornerRadius, theme: &Theme) {
    let outer = CornerRadius::same(radius.nw.saturating_add(3));
    painter.rect_stroke(
        rect.expand(1.5),
        outer,
        Stroke::new(3.0, theme.ring_soft()),
        egui::StrokeKind::Outside,
    );
    painter.rect_stroke(rect, radius, Stroke::new(1.0, theme.ring), egui::StrokeKind::Inside);
}
