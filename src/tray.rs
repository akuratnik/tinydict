use ksni::{Icon, Tray, TrayMethods};

#[derive(Clone, Copy)]
pub enum Color {
    Red,
    Amber,
}

pub struct Handle {
    inner: ksni::Handle<Dot>,
}

struct Dot {
    color: Color,
}

impl Tray for Dot {
    fn id(&self) -> String {
        "tinydict".into()
    }
    fn title(&self) -> String {
        match self.color {
            Color::Red => "tinydict — recording",
            Color::Amber => "tinydict — finishing",
        }
        .into()
    }
    fn icon_pixmap(&self) -> Vec<Icon> {
        vec![dot_icon(self.color)]
    }
}

pub async fn start(color: Color) -> Option<Handle> {
    match (Dot { color }).spawn().await {
        Ok(inner) => Some(Handle { inner }),
        Err(err) => {
            eprintln!("tinydict: tray unavailable, continuing without it ({err})");
            None
        }
    }
}

impl Handle {
    pub async fn set(&self, color: Color) {
        self.inner.update(|dot| dot.color = color).await;
    }

    pub async fn shutdown(self) {
        self.inner.shutdown().await;
    }
}

fn dot_icon(color: Color) -> Icon {
    const SIZE: i32 = 22;
    let (r, g, b) = match color {
        Color::Red => (0xE0, 0x22, 0x22),
        Color::Amber => (0xE0, 0xA0, 0x18),
    };
    let mut data = vec![0u8; (SIZE * SIZE * 4) as usize];
    let cx = (SIZE as f32 - 1.0) / 2.0;
    let rad2 = 8.0 * 8.0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as f32 - cx;
            let dy = y as f32 - cx;
            if dx * dx + dy * dy <= rad2 {
                let i = ((y * SIZE + x) * 4) as usize;
                data[i] = 255;
                data[i + 1] = r;
                data[i + 2] = g;
                data[i + 3] = b;
            }
        }
    }
    Icon {
        width: SIZE,
        height: SIZE,
        data,
    }
}
