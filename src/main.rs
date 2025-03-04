use xcb::{x, Connection, Xid, XidNew};
use std::collections::HashMap;

mod atoms {
    pub const NET_CLIENT_LIST: &[u8] = b"_NET_CLIENT_LIST";
    pub const NET_WM_ICON: &[u8] = b"_NET_WM_ICON";
    pub const WM_DELETE_WINDOW: &[u8] = b"WM_DELETE_WINDOW";
    pub const NET_WM_WINDOW_TYPE: &[u8] = b"_NET_WM_WINDOW_TYPE";
    pub const NET_WM_WINDOW_TYPE_NORMAL: &[u8] = b"_NET_WM_WINDOW_TYPE_NORMAL";
    pub const NET_WM_STATE_SKIP_TASKBAR: &[u8] = b"_NET_WM_STATE_SKIP_TASKBAR";
    pub const WM_PROTOCOLS: &[u8] = b"WM_PROTOCOLS";
    pub const _NET_WM_NAME: &[u8] = b"_NET_WM_NAME";
    pub const UTF8_STRING: &[u8] = b"UTF8_STRING";
}

const ICON_SIZE: u16 = 32;
const PANEL_WIDTH: u16 = 32;
const ITEM_SPACING: i16 = 4;

struct Atoms {
    net_client_list: x::Atom,
    net_wm_icon: x::Atom,
    wm_delete: x::Atom,
    net_wm_window_type: x::Atom,
    net_wm_window_type_normal: x::Atom,
    net_wm_state_skip_taskbar: x::Atom,
    net_wm_name: x::Atom,
    utf8_string: x::Atom,
}

#[derive(Debug, Clone)]
struct WindowData {
    icon_pixmap: Option<x::Pixmap>,
    icon_rect: x::Rectangle,
    title: String,
}

struct Panel {
    window: x::Window,
    gc: x::Gcontext,
    icon_gc: x::Gcontext,
    height: u16,
    windows: HashMap<x::Window, WindowData>,
    atoms: Atoms,
}

impl Panel {
    fn new(conn: &Connection, screen: &x::Screen) -> Self {
        let window = conn.generate_id();
        let gc = conn.generate_id();
        let icon_gc = conn.generate_id();
        let screen_height = screen.height_in_pixels();

        let atoms = Atoms {
            net_client_list: intern_atom(conn, atoms::NET_CLIENT_LIST),
            net_wm_icon: intern_atom(conn, atoms::NET_WM_ICON),
            wm_delete: intern_atom(conn, atoms::WM_DELETE_WINDOW),
            net_wm_window_type: intern_atom(conn, atoms::NET_WM_WINDOW_TYPE),
            net_wm_window_type_normal: intern_atom(conn, atoms::NET_WM_WINDOW_TYPE_NORMAL),
            net_wm_state_skip_taskbar: intern_atom(conn, atoms::NET_WM_STATE_SKIP_TASKBAR),
            net_wm_name: intern_atom(conn, atoms::_NET_WM_NAME),
            utf8_string: intern_atom(conn, atoms::UTF8_STRING),
        };

        let panel_x = (screen.width_in_pixels() - PANEL_WIDTH) as i16;

        // Создание окна с чёрным фоном
        conn.send_request(&x::CreateWindow {
            depth: screen.root_depth() as u8,
            wid: window,
            parent: screen.root(),
            x: panel_x,
            y: 0,
            width: PANEL_WIDTH,
            height: screen_height,
            border_width: 0,
            class: x::WindowClass::InputOutput,
            visual: screen.root_visual(),
            value_list: &[
                x::Cw::BackPixel(screen.black_pixel()),
                x::Cw::OverrideRedirect(true),
                x::Cw::EventMask(
                    x::EventMask::EXPOSURE 
                    | x::EventMask::BUTTON_PRESS 
                    | x::EventMask::PROPERTY_CHANGE
                ),
            ],
        });

        // Графический контекст
        conn.send_request(&x::CreateGc {
            cid: gc,
            drawable: x::Drawable::Window(window),
            value_list: &[
                x::Gc::Foreground(screen.black_pixel()),
                x::Gc::Background(screen.black_pixel()),
                x::Gc::GraphicsExposures(false),
            ],
        });

        // Графический контекст для иконок
        conn.send_request(&x::CreateGc {
            cid: icon_gc,
            drawable: x::Drawable::Window(window),
            value_list: &[
                x::Gc::SubwindowMode(x::SubwindowMode::IncludeInferiors),
                x::Gc::GraphicsExposures(false),
            ],
        });

        Panel {
            window,
            gc,
            icon_gc,
            height: screen_height,
            windows: HashMap::new(),
            atoms,
        }
    }

    fn update_windows(&mut self, conn: &Connection) {
        let root = conn.get_setup().roots().next().unwrap().root();
        let cookie = conn.send_request(&x::GetProperty {
            delete: false,
            window: root,
            property: self.atoms.net_client_list,
            r#type: x::ATOM_WINDOW,
            long_offset: 0,
            long_length: 8192,
        });

        if let Ok(reply) = conn.wait_for_reply(cookie) {
            if reply.format() == 32 {
                let current_windows: Vec<x::Window> = reply.value::<u32>()
                    .iter()
                    .map(|&id| unsafe { x::Window::new(id) })
                    .collect();

                self.windows.retain(|k, v| current_windows.contains(k) && v.icon_pixmap.is_some());
                
                for &window in &current_windows {
                    if self.should_show_window(conn, window) {
                        let title = get_window_title(conn, window, &self.atoms);
                        let icon = load_window_icon(conn, window, self.atoms.net_wm_icon, self.icon_gc);
                        
                        if let Some(icon) = icon {
                            self.windows.entry(window).or_insert(WindowData {
                                icon_pixmap: Some(icon),
                                icon_rect: x::Rectangle { x: 0, y: 0, width: 0, height: 0 },
                                title,
                            });
                        }
                    }
                }
                
                self.update_layout();
            }
        }
    }

    fn should_show_window(&self, conn: &Connection, window: x::Window) -> bool {
        self.is_normal_window(conn, window) &&
        !self.is_skip_taskbar(conn, window) &&
        !self.is_override_redirect(conn, window)
    }

    fn is_normal_window(&self, conn: &Connection, window: x::Window) -> bool {
        let cookie = conn.send_request(&x::GetProperty {
            delete: false,
            window,
            property: self.atoms.net_wm_window_type,
            r#type: x::ATOM_ATOM,
            long_offset: 0,
            long_length: 32,
        });

        conn.wait_for_reply(cookie)
            .map(|r| r.value::<x::Atom>().contains(&self.atoms.net_wm_window_type_normal))
            .unwrap_or(false)
    }

    fn is_skip_taskbar(&self, conn: &Connection, window: x::Window) -> bool {
        let cookie = conn.send_request(&x::GetProperty {
            delete: false,
            window,
            property: self.atoms.net_wm_state_skip_taskbar,
            r#type: x::ATOM_ATOM,
            long_offset: 0,
            long_length: 32,
        });

        conn.wait_for_reply(cookie)
            .map(|r| !r.value::<x::Atom>().is_empty())
            .unwrap_or(false)
    }

    fn is_override_redirect(&self, conn: &Connection, window: x::Window) -> bool {
        conn.wait_for_reply(conn.send_request(&x::GetWindowAttributes { window }))
            .map(|attrs| attrs.override_redirect())
            .unwrap_or(true)
    }

    fn update_layout(&mut self) {
        let mut y_pos = ITEM_SPACING;
        let x_center = (PANEL_WIDTH as i16 - ICON_SIZE as i16) / 2;

        self.windows.retain(|_, v| v.icon_pixmap.is_some());

        for data in self.windows.values_mut() {
            data.icon_rect = x::Rectangle {
                x: x_center,
                y: y_pos,
                width: ICON_SIZE,
                height: ICON_SIZE,
            };
            y_pos += ICON_SIZE as i16 + ITEM_SPACING;
        }
    }

    fn redraw(&self, conn: &Connection) {
        // Очистка фона
        conn.send_request(&x::PolyFillRectangle {
            drawable: x::Drawable::Window(self.window),
            gc: self.gc,
            rectangles: &[x::Rectangle {
                x: 0,
                y: 0,
                width: PANEL_WIDTH,
                height: self.height,
            }],
        });

        // Рисование иконок
        for data in self.windows.values() {
            if let Some(pixmap) = data.icon_pixmap {
                conn.send_request(&x::CopyArea {
                    src_drawable: x::Drawable::Pixmap(pixmap),
                    dst_drawable: x::Drawable::Window(self.window),
                    gc: self.icon_gc,
                    src_x: 0,
                    src_y: 0,
                    dst_x: data.icon_rect.x,
                    dst_y: data.icon_rect.y,
                    width: data.icon_rect.width,
                    height: data.icon_rect.height,
                });
            }
        }

        conn.flush().unwrap();
    }
}

fn intern_atom(conn: &Connection, name: &[u8]) -> x::Atom {
    conn.wait_for_reply(conn.send_request(&x::InternAtom {
        only_if_exists: true,
        name,
    })).map(|r| r.atom()).unwrap_or(x::ATOM_NONE)
}

fn get_window_title(conn: &Connection, window: x::Window, atoms: &Atoms) -> String {
    let cookie = conn.send_request(&x::GetProperty {
        delete: false,
        window,
        property: atoms.net_wm_name,
        r#type: atoms.utf8_string,
        long_offset: 0,
        long_length: 256,
    });
    
    if let Ok(reply) = conn.wait_for_reply(cookie) {
        if reply.format() == 8 {
            return String::from_utf8_lossy(reply.value()).into_owned();
        }
    }

    "Untitled".to_string()
}

fn load_window_icon(
    conn: &Connection,
    window: x::Window,
    atom: x::Atom,
    icon_gc: x::Gcontext,
) -> Option<x::Pixmap> {
    let cookie = conn.send_request(&x::GetProperty {
        delete: false,
        window,
        property: atom,
        r#type: x::ATOM_CARDINAL,
        long_offset: 0,
        long_length: 0,
    });

    let reply = conn.wait_for_reply(cookie).ok()?;
    let bytes_needed = reply.bytes_after() as usize;
    let long_length = (bytes_needed + 3) / 4;

    let cookie = conn.send_request(&x::GetProperty {
        delete: false,
        window,
        property: atom,
        r#type: x::ATOM_CARDINAL,
        long_offset: 0,
        long_length: long_length as u32,
    });

    let reply = conn.wait_for_reply(cookie).ok()?;
    let raw_data = reply.value::<u32>();
    
    if raw_data.len() < 2 {
        return None;
    }

    let (width, height, icon_data) = find_best_icon(raw_data)?;
    let screen = conn.get_setup().roots().next()?;
    
    let pixmap = conn.generate_id();
    conn.send_request(&x::CreatePixmap {
        depth: 24,
        pid: pixmap,
        drawable: x::Drawable::Window(screen.root()),
        width: ICON_SIZE,
        height: ICON_SIZE,
    });

    let mut pixels = Vec::with_capacity(ICON_SIZE as usize * ICON_SIZE as usize * 4);
    for y in 0..ICON_SIZE {
        for x in 0..ICON_SIZE {
            let src_x = (x as f32 * width as f32 / ICON_SIZE as f32) as usize;
            let src_y = (y as f32 * height as f32 / ICON_SIZE as f32) as usize;
            let idx = src_y * width + src_x;
            
            let pixel = *icon_data.get(idx).unwrap_or(&0);
            pixels.extend_from_slice(&[
                ((pixel >> 16) & 0xFF) as u8, // R
                ((pixel >> 8) & 0xFF) as u8,  // G
                (pixel & 0xFF) as u8,         // B
                0xFF,                         // A
            ]);
        }
    }

    conn.send_request(&x::PutImage {
        format: x::ImageFormat::ZPixmap,
        drawable: x::Drawable::Pixmap(pixmap),
        gc: icon_gc,
        width: ICON_SIZE,
        height: ICON_SIZE,
        dst_x: 0,
        dst_y: 0,
        left_pad: 0,
        depth: 24,
        data: &pixels,
    });

    Some(pixmap)
}

fn find_best_icon(data: &[u32]) -> Option<(usize, usize, &[u32])> {
    let mut best_size = 0;
    let mut best_icon = None;
    let mut offset = 0;

    while offset + 1 < data.len() {
        let width = data[offset] as usize;
        let height = data[offset + 1] as usize;
        let icon_size = width * height;
        let required_length = offset + 2 + icon_size;

        if required_length > data.len() {
            break;
        }

        if icon_size > best_size && width <= 256 && height <= 256 {
            best_size = icon_size;
            best_icon = Some((width, height, &data[offset + 2..required_length]));
        }

        offset = required_length;
    }

    best_icon
}

fn main() {
    let (conn, screen_num) = Connection::connect(None).unwrap();
    let screen = &conn.get_setup().roots().nth(screen_num as usize).unwrap();
    
    let mut panel = Panel::new(&conn, screen);
    
    conn.send_request(&x::MapWindow { window: panel.window });
    conn.flush().unwrap();

    let wm_protocols = intern_atom(&conn, atoms::WM_PROTOCOLS);
    conn.send_request(&x::ChangeProperty {
        mode: x::PropMode::Replace,
        window: panel.window,
        property: wm_protocols,
        r#type: x::ATOM_ATOM,
        data: &[panel.atoms.wm_delete],
    });

    loop {
        panel.update_windows(&conn);
        panel.redraw(&conn);

        if let Ok(event) = conn.wait_for_event() {
            match event {
                xcb::Event::X(x::Event::Expose(ev)) => {
                    if ev.window() == panel.window {
                        panel.redraw(&conn);
                    }
                }
                xcb::Event::X(x::Event::ButtonPress(ev)) => {
                    if ev.event() == panel.window {
                        let (x, y) = (ev.event_x(), ev.event_y());
                        println!("Clicked at ({}, {})", x, y);
                        
                        for (win, data) in &panel.windows {
                            let rect = data.icon_rect;
                            if x >= rect.x 
                                && x <= rect.x + rect.width as i16
                                && y >= rect.y 
                                && y <= rect.y + rect.height as i16 
                            {
                                println!("Switching to window: {} (ID: {:?})", data.title, win);
                                focus_window(&conn, *win);
                            }
                        }
                    }
                }
                xcb::Event::X(x::Event::ClientMessage(ev)) => {
                    use xcb::x::ClientMessageData;
                    if let ClientMessageData::Data32(data) = ev.data() {
                        if data[0] == panel.atoms.wm_delete.resource_id() {
                            break;
                        }
                    }
                }
                _ => {}
            }
        }
    }

    conn.send_request(&x::DestroyWindow { window: panel.window });
    conn.flush().unwrap();
}

fn focus_window(conn: &Connection, window: x::Window) {
    let cookie = conn.send_request(&x::GetWindowAttributes { window });
    if let Ok(attrs) = conn.wait_for_reply(cookie) {
        if !attrs.override_redirect() && attrs.map_state() == x::MapState::Viewable {
            conn.send_request(&x::SetInputFocus {
                revert_to: x::InputFocus::PointerRoot,
                focus: window,
                time: x::CURRENT_TIME,
            });
            
            conn.send_request(&x::ConfigureWindow {
                window,
                value_list: &[x::ConfigWindow::StackMode(x::StackMode::Above)],
            });
            conn.flush().unwrap();
        }
    }
}
