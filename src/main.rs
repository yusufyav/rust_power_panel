use gtk4::prelude::*;
use gtk4::{
    glib, Application, ApplicationWindow, Box as GtkBox, Label, Orientation, CssProvider,
};
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use nvml_wrapper::Nvml;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use sysinfo::{Components, System};

const APP_ID: &str = "com.rustpanel.powerpanel";

#[derive(Clone, Default)]
struct SensorData {
    cpu_temp: f32,
    cpu_watt: f32,
    gpu_temp: u32,
    gpu_watt: f32,
    gpu_dec:  u32,
    gpu_enc:  u32,
}

fn main() -> glib::ExitCode {
    let app = Application::builder().application_id(APP_ID).build();
    app.connect_activate(build_ui);
    app.run()
}

fn build_ui(app: &Application) {
    let window = ApplicationWindow::builder()
        .application(app)
        .default_width(360)
        .default_height(1)
        .decorated(false)
        .build();

    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Right, true);
    window.set_margin(Edge::Top, 60);
    window.set_margin(Edge::Right, 20);

    let css = CssProvider::new();
    css.load_from_data("
        window { background-color: transparent; }
        .panel {
            background-color: rgba(10, 10, 10, 0.97);
            border-radius: 18px;
            border: 1px solid rgba(255, 255, 255, 0.15);
            padding: 18px 24px;
        }
        .total-watt {
            color: #00ffcc;
            font-family: 'JetBrains Mono', monospace;
            font-size: 26px;
            font-weight: bold;
        }
        .row-label-cpu {
            color: #ff9f43;
            font-family: 'JetBrains Mono', monospace;
            font-size: 16px;
            font-weight: bold;
        }
        .row-label-gpu {
            color: #2ecc71;
            font-family: 'JetBrains Mono', monospace;
            font-size: 16px;
            font-weight: bold;
        }
        .row-label-dec {
            color: #a29bfe;
            font-family: 'JetBrains Mono', monospace;
            font-size: 16px;
            font-weight: bold;
        }
        .row-label-enc {
            color: #74b9ff;
            font-family: 'JetBrains Mono', monospace;
            font-size: 16px;
            font-weight: bold;
        }
        .row-watt {
            color: #ffffff;
            font-family: 'JetBrains Mono', monospace;
            font-size: 16px;
        }
        .row-temp-icon {
            color: #ff4757;
            font-family: 'JetBrains Mono', monospace;
            font-size: 16px;
        }
        .row-temp-value {
            color: #ff4757;
            font-family: 'JetBrains Mono', monospace;
            font-size: 16px;
        }
        .row-percent {
            color: #dfe6e9;
            font-family: 'JetBrains Mono', monospace;
            font-size: 16px;
        }
        .divider {
            background-color: rgba(255, 255, 255, 0.10);
            min-height: 1px;
            margin: 4px 0px;
        }
    ");

    gtk4::style_context_add_provider_for_display(
        &gtk4::gdk::Display::default().unwrap(),
        &css,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    let panel = GtkBox::new(Orientation::Vertical, 8);
    panel.add_css_class("panel");

    let total_label = Label::new(Some("⚡  0.0 W"));
    total_label.add_css_class("total-watt");
    total_label.set_halign(gtk4::Align::Center);
    panel.append(&total_label);

    let (cpu_row, cpu_watt_lbl, cpu_temp_lbl) = make_hw_row("⚙ CPU", "row-label-cpu");
    panel.append(&cpu_row);

    let (gpu_row, gpu_watt_lbl, gpu_temp_lbl) = make_hw_row("▣ GPU", "row-label-gpu");
    panel.append(&gpu_row);

    let sep = gtk4::Separator::new(Orientation::Horizontal);
    sep.add_css_class("divider");
    panel.append(&sep);

    let (dec_row, dec_val_lbl) = make_util_row("◈ DEC", "row-label-dec");
    panel.append(&dec_row);

    let (enc_row, enc_val_lbl) = make_util_row("◉ ENC", "row-label-enc");
    panel.append(&enc_row);

    window.set_child(Some(&panel));

    let gesture = gtk4::GestureClick::new();
    gesture.set_button(3);
    let win_clone = window.clone();
    gesture.connect_released(move |_, _, _, _| {
        win_clone.close();
    });
    window.add_controller(gesture);

    window.present();

    let data: Arc<Mutex<SensorData>> = Arc::new(Mutex::new(SensorData::default()));
    let data_writer = data.clone();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            let mut sys = System::new_all();
            let nvml = Nvml::init().ok();

            loop {
                let components = Components::new_with_refreshed_list();
                let mut cpu_temp = 0.0f32;
                for c in &components {
                    let lbl = c.label().to_lowercase();
                    if lbl.contains("cpu") || lbl.contains("k10") || lbl.contains("composite") {
                        let t = c.temperature();
                        if t > cpu_temp { cpu_temp = t; }
                    }
                }

                let cpu_watt = read_rapl_watts().unwrap_or_else(|| {
                    sys.refresh_cpu_usage();
                    let avg: f32 = sys.cpus().iter().map(|c| c.cpu_usage()).sum::<f32>()
                        / sys.cpus().len() as f32;
                    30.0 + avg * 1.4
                });

                let mut gpu_watt = 0.0f32;
                let mut gpu_temp = 0u32;
                let mut gpu_dec  = 0u32;
                let mut gpu_enc  = 0u32;

                if let Some(ref n) = nvml {
                    if let Ok(dev) = n.device_by_index(0) {
                        gpu_watt = dev.power_usage().unwrap_or(0) as f32 / 1000.0;
                        gpu_temp = dev.temperature(
                            nvml_wrapper::enum_wrappers::device::TemperatureSensor::Gpu
                        ).unwrap_or(0);

                        if let Ok(info) = dev.decoder_utilization() {
                            gpu_dec = info.utilization;
                        }
                        if let Ok(info) = dev.encoder_utilization() {
                            gpu_enc = info.utilization;
                        }
                    }
                }

                if let Ok(mut d) = data_writer.lock() {
                    d.cpu_temp = cpu_temp;
                    d.cpu_watt = cpu_watt;
                    d.gpu_temp = gpu_temp;
                    d.gpu_watt = gpu_watt;
                    d.gpu_dec  = gpu_dec;
                    d.gpu_enc  = gpu_enc;
                }

                tokio::time::sleep(Duration::from_millis(1000)).await;
            }
        });
    });

    glib::timeout_add_local(Duration::from_millis(1000), move || {
        if let Ok(d) = data.lock() {
            let total = d.cpu_watt + d.gpu_watt;
            total_label.set_text(&format!("⚡  {:.1} W", total));
            cpu_watt_lbl.set_text(&format!("{:.1} W", d.cpu_watt));
            cpu_temp_lbl.set_text(&format!("{:.0} °C", d.cpu_temp));
            gpu_watt_lbl.set_text(&format!("{:.1} W", d.gpu_watt));
            gpu_temp_lbl.set_text(&format!("{} °C", d.gpu_temp));
            dec_val_lbl.set_text(&format!("NVDEC  {} %", d.gpu_dec));
            enc_val_lbl.set_text(&format!("NVENC  {} %", d.gpu_enc));
        }
        glib::ControlFlow::Continue
    });
}

fn make_hw_row(label_text: &str, label_class: &str) -> (GtkBox, Label, Label) {
    let row = GtkBox::new(Orientation::Horizontal, 0);

    let name_lbl = Label::new(Some(label_text));
    name_lbl.add_css_class(label_class);
    name_lbl.set_width_chars(8);
    name_lbl.set_xalign(0.0);

    let watt_lbl = Label::new(Some("  0.0 W"));
    watt_lbl.add_css_class("row-watt");
    watt_lbl.set_width_chars(9);
    watt_lbl.set_xalign(1.0);

    let temp_icon = Label::new(Some("  🌡"));
    temp_icon.add_css_class("row-temp-icon");
    temp_icon.set_width_chars(3);
    temp_icon.set_xalign(1.0);

    let temp_lbl = Label::new(Some("  0 °C"));
    temp_lbl.add_css_class("row-temp-value");
    temp_lbl.set_width_chars(8);
    temp_lbl.set_xalign(1.0);

    row.append(&name_lbl);
    row.append(&watt_lbl);
    row.append(&temp_icon);
    row.append(&temp_lbl);

    (row, watt_lbl, temp_lbl)
}

fn make_util_row(label_text: &str, label_class: &str) -> (GtkBox, Label) {
    let row = GtkBox::new(Orientation::Horizontal, 0);

    let name_lbl = Label::new(Some(label_text));
    name_lbl.add_css_class(label_class);
    name_lbl.set_width_chars(8);
    name_lbl.set_xalign(0.0);

    let val_lbl = Label::new(Some("  0 %"));
    val_lbl.add_css_class("row-percent");
    val_lbl.set_hexpand(true);
    val_lbl.set_xalign(1.0);

    row.append(&name_lbl);
    row.append(&val_lbl);

    (row, val_lbl)
}

fn read_rapl_watts() -> Option<f32> {
    use std::fs;
    use std::time::Instant;

    let path = "/sys/class/powercap/amd-energy-pkg/energy_uj";
    let e1: u64 = fs::read_to_string(path).ok()?.trim().parse().ok()?;
    let t1 = Instant::now();
    std::thread::sleep(Duration::from_millis(100));
    let e2: u64 = fs::read_to_string(path).ok()?.trim().parse().ok()?;
    let dt = t1.elapsed().as_secs_f32();
    Some((e2.saturating_sub(e1)) as f32 / dt / 1_000_000.0)
}