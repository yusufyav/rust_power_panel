// ── GUI stilleri ────────────────────────────────────────────────────────────
// Çalışma zamanında geçiş yapılabilen panel stilleri. Yeni stil eklemek:
// variant + name/stack_name/next match kolları + bir build_*_content fonksiyonu.

#[derive(Clone, Copy, PartialEq)]
pub(super) enum GuiStyle {
    Classic,
    Bars,
}

impl GuiStyle {
    fn name(self) -> &'static str {
        match self {
            GuiStyle::Classic => "Classic",
            GuiStyle::Bars => "Bars",
        }
    }

    pub(super) fn label(self) -> String {
        format!("⟳ {}", self.name())
    }

    pub(super) fn stack_name(self) -> &'static str {
        match self {
            GuiStyle::Classic => "classic",
            GuiStyle::Bars => "bars",
        }
    }

    pub(super) fn next(self) -> Self {
        match self {
            GuiStyle::Classic => GuiStyle::Bars,
            GuiStyle::Bars => GuiStyle::Classic,
        }
    }
}

// Tüm stillerin CSS'i tek provider'da. Kart arka planı paylaşılan .panel-card'da;
// çakışan içerik sınıfları (.total-watt, .lbl-cpu, .val-pct ...) her stilde farklı
// boyutta olduğundan .panel / .panel2 ebeveyni altında kapsamlanır.
pub(super) const PANEL_CSS: &str = "
    window { background-color: transparent; }

    .panel-card {
        background-color: rgba(10, 10, 10, 0.82);
        border-radius: 18px;
        border: 1px solid rgba(255, 255, 255, 0.15);
        padding: 14px 18px;
    }

    .style-toggle {
        color: #a0a8b0;
        font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 11px;
        background-image: none;
        background-color: rgba(255, 255, 255, 0.06);
        border: 1px solid rgba(255, 255, 255, 0.12);
        border-radius: 8px;
        box-shadow: none;
        padding: 1px 8px;
        min-height: 0;
        min-width: 0;
    }
    .style-toggle:hover {
        background-image: none;
        background-color: rgba(255, 255, 255, 0.14);
    }

    /* ── Paylaşılan process tablosu ── */
    .proc-hdr {
        color: #a29bfe; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 12px; font-weight: bold;
    }
    .proc-val {
        color: #b2bec3; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 12px;
    }
    .proc-num {
        color: #dfe6e9; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 12px;
    }
    .divider {
        background-color: rgba(255, 255, 255, 0.10);
        min-height: 1px; margin: 4px 0px;
    }

    /* ── Classic (.panel) ── */
    .panel .total-watt {
        color: #00ffcc; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 26px; font-weight: bold;
    }
    .panel .lbl-cpu {
        color: #ff9f43; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 16px; font-weight: bold;
    }
    .panel .lbl-gpu {
        color: #2ecc71; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 16px; font-weight: bold;
    }
    .panel .val-watt {
        color: #ffffff; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 16px;
    }
    .panel .val-temp-cool {
        color: #4cd964; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 16px;
    }
    .panel .val-temp-warm {
        color: #ff9f43; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 16px;
    }
    .panel .val-temp-hot {
        color: #ff4757; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 16px;
    }
    .panel .lbl-ram {
        color: #00cec9; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 16px; font-weight: bold;
    }
    .panel .val-vram {
        color: #74b9ff; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 14px;
    }
    .panel .val-pct {
        color: #dfe6e9; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 16px;
    }
    .panel .hw-icon {
        font-size: 22px; font-weight: bold;
    }

    /* ── Bars (.panel2) ── */
    .panel2 .brand-lbl {
        color: #a0a8b0; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 13px;
    }
    .panel2 .brand-icon {
        color: #a0a8b0; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 24px; font-weight: bold;
    }
    .panel2 .total-watt {
        color: #00ffcc; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 22px; font-weight: bold;
    }
    .panel2 .lbl-cpu {
        color: #ff9f43; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 14px; font-weight: bold;
    }
    .panel2 .lbl-gpu {
        color: #2ecc71; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 14px; font-weight: bold;
    }
    .panel2 .lbl-ram {
        color: #00cec9; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 14px; font-weight: bold;
    }
    .panel2 .val-pct {
        color: #dfe6e9; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 13px;
    }
    .panel2 .stat-lbl {
        font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 13px;
    }
";

pub(super) fn temp_css_class(temp: f32) -> &'static str {
    if temp >= 80.0 {
        "val-temp-hot"
    } else if temp >= 60.0 {
        "val-temp-warm"
    } else {
        "val-temp-cool"
    }
}

pub(super) fn usage_css_class(pct: u32) -> &'static str {
    if pct >= 90 {
        "val-temp-hot"
    } else if pct >= 75 {
        "val-temp-warm"
    } else {
        "val-pct"
    }
}

pub(super) fn temp_hex_color(t: f32) -> &'static str {
    if t >= 80.0 {
        "#ff4757"
    } else if t >= 60.0 {
        "#ff9f43"
    } else {
        "#4cd964"
    }
}

pub(super) fn make_style_toggle(label: &str) -> gtk4::Button {
    gtk4::Button::builder()
        .label(label)
        .css_classes(vec!["style-toggle".to_string()])
        .halign(gtk4::Align::Start)
        .build()
}
