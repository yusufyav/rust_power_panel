mod bars;
mod classic;
mod process;
mod style;
mod worker;

use bars::build_bars_content;
use classic::build_classic_content;
use process::build_proc_section;
use style::{make_style_toggle, GuiStyle, PANEL_CSS};
use worker::spawn_sensor_thread;

use crate::types::SensorData;
use crate::APP_ID;
use gtk4::prelude::*;
use gtk4::{glib, Application, ApplicationWindow, Box as GtkBox, CssProvider, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

// Bir stil içeriği: kök widget + her-tick güncelleme closure'ı.
type Content = (GtkBox, Box<dyn Fn(&SensorData)>);

const GUI_FIRST_UPDATE_DELAY: Duration = Duration::from_millis(300);

// ── Stil-geçişli pencere (ortak iskelet) ─────────────────────────────────────

fn build_switchable_ui(app: &Application, initial: GuiStyle, interval: Duration) {
    let window = ApplicationWindow::builder()
        .application(app)
        .default_width(340)
        .default_height(1)
        .decorated(false)
        .build();

    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Right, true);
    window.set_margin(Edge::Top, 60);
    window.set_margin(Edge::Right, 20);
    window.set_keyboard_mode(KeyboardMode::None);

    let css = CssProvider::new();
    css.load_from_data(PANEL_CSS);
    gtk4::style_context_add_provider_for_display(
        &gtk4::gdk::Display::default().unwrap(),
        &css,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    // Tek sensör thread'i + tek paylaşılan veri.
    let data = Arc::new(Mutex::new(SensorData::default()));
    spawn_sensor_thread(data.clone(), interval);

    // Kart: sabit üst kontrol (stil toggle) + stack içerik + paylaşılan process bölümü.
    let card = GtkBox::new(Orientation::Vertical, 8);
    card.add_css_class("panel-card");
    card.set_size_request(340, -1);

    // Üst kontrol çubuğu: stil toggle (her iki stilde aynı konumda, sabit).
    let header = GtkBox::new(Orientation::Horizontal, 0);
    let style_toggle = make_style_toggle(&initial.label());
    header.append(&style_toggle);
    card.append(&header);

    // İçerikler bir kez inşa edilir.
    let (classic_root, classic_update) = build_classic_content();
    let (bars_root, bars_update) = build_bars_content();

    let stack = gtk4::Stack::new();
    stack.set_transition_type(gtk4::StackTransitionType::None);
    stack.set_hhomogeneous(false);
    stack.set_vhomogeneous(false);
    stack.add_named(&classic_root, Some(GuiStyle::Classic.stack_name()));
    stack.add_named(&bars_root, Some(GuiStyle::Bars.stack_name()));
    stack.set_visible_child_name(initial.stack_name());
    card.append(&stack);

    // Paylaşılan process bölümü (manuel açılır), stilden bağımsız.
    let (proc_section, proc_update) = build_proc_section();
    card.append(&proc_section);

    // Stil toggle: görünen içeriği değiştir, etiketini güncelle.
    let style = Rc::new(Cell::new(initial));
    {
        let stack = stack.clone();
        let style = style.clone();
        let style_toggle_lbl = style_toggle.clone();
        style_toggle.connect_clicked(move |_| {
            let next = style.get().next();
            style.set(next);
            stack.set_visible_child_name(next.stack_name());
            style_toggle_lbl.set_label(&next.label());
        });
    }

    window.set_child(Some(&card));

    // Sağ tık → kapat
    let gesture = gtk4::GestureClick::new();
    gesture.set_button(3);
    let win_clone = window.clone();
    gesture.connect_released(move |_, _, _, _| win_clone.close());
    window.add_controller(gesture);

    // Tekerlek → opaklık
    let opacity = Rc::new(Cell::new(1.0_f64));
    let scroll = gtk4::EventControllerScroll::new(gtk4::EventControllerScrollFlags::VERTICAL);
    let win_op = window.clone();
    let op_cell = opacity.clone();
    scroll.connect_scroll(move |_, _, dy| {
        let new = (op_cell.get() - dy * 0.05).clamp(0.3, 1.0);
        op_cell.set(new);
        win_op.set_opacity(new);
        glib::Propagation::Stop
    });
    window.add_controller(scroll);

    window.present();

    // İlk güncelleme kısa prime sonrası, devamı ayarlı interval ile yapılır.
    let mut update_state = Some((data, classic_update, bars_update, proc_update));
    glib::timeout_add_local(GUI_FIRST_UPDATE_DELAY, move || {
        let Some((data, classic_update, bars_update, proc_update)) = update_state.take() else {
            return glib::ControlFlow::Break;
        };

        if let Ok(d) = data.lock() {
            let target = d.clone();
            classic_update(&target);
            bars_update(&target);
            proc_update(&target);
        }

        glib::timeout_add_local(interval, move || {
            let target = match data.lock() {
                Ok(d) => d.clone(),
                Err(_) => return glib::ControlFlow::Continue,
            };
            classic_update(&target);
            bars_update(&target);
            proc_update(&target);
            glib::ControlFlow::Continue
        });

        glib::ControlFlow::Break
    });
}

pub(crate) fn run_gui(interval: Duration, args: &[String]) -> glib::ExitCode {
    let app = Application::builder().application_id(APP_ID).build();
    app.connect_activate(move |app| build_switchable_ui(app, GuiStyle::Classic, interval));
    let argv0 = args.first().map(String::as_str).unwrap_or("power_panel");
    app.run_with_args(&[argv0])
}

pub(crate) fn run_gui2(interval: Duration, args: &[String]) -> glib::ExitCode {
    let app = Application::builder().application_id(APP_ID).build();
    app.connect_activate(move |app| build_switchable_ui(app, GuiStyle::Bars, interval));
    let argv0 = args.first().map(String::as_str).unwrap_or("power_panel");
    app.run_with_args(&[argv0])
}
