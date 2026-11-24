use super::Expression;
use crate::anim::{lerp, smoothstep};
use crate::paint::Card;
use eframe::egui::{Color32, Mesh, Pos2, Shape, Vec2};
use std::f32::consts::{PI, TAU};

const PINK: Color32 = Color32::from_rgb(251, 144, 186);
const GOLD: Color32 = Color32::from_rgb(255, 216, 117);
const LILAC: Color32 = Color32::from_rgb(188, 168, 255);

// Half-width, half-height and the rise/slope of the eye's centre line, in island points.
#[derive(Clone, Copy)]
struct Eye {
    w: f32,
    h: f32,
    arch: f32,
    tilt: f32,
    y: f32,
}

impl Eye {
    const OPEN: Self = Self { w: 1.8, h: 4.0, arch: 0.0, tilt: 0.0, y: 0.0 };
    const SMILE: Self = Self { w: 3.5, h: 0.96, arch: -2.2, tilt: 0.0, y: 0.8 };
    const SOFT: Self = Self { arch: 1.5, y: -0.3, ..Self::SMILE };
    const SLANT: Self = Self { w: 3.1, h: 1.36, arch: 0.0, tilt: -0.8, y: 0.0 };

    fn toward(self, target: Self, t: f32) -> Self {
        Self {
            w: lerp(self.w, target.w, t), h: lerp(self.h, target.h, t),
            arch: lerp(self.arch, target.arch, t), tilt: lerp(self.tilt, target.tilt, t),
            y: lerp(self.y, target.y, t),
        }
    }

    fn shape(self, c: &Card, x: f32, y: f32) -> Shape {
        const COLUMNS: usize = 16;
        let mut outline = [Pos2::ZERO; COLUMNS * 2];
        let radius = self.w.min(self.h);
        for i in 0..COLUMNS {
            let t = -(PI * (i as f32 + 0.5) / COLUMNS as f32).cos();
            let dx = t * self.w;
            let corner = (dx.abs() - (self.w - radius)).max(0.0);
            let half_height = self.h - radius + (radius * radius - corner * corner).max(0.0).sqrt();
            let middle = y + self.y + self.arch * (1.0 - t * t) + self.tilt * t;
            outline[i] = c.pos(x + dx, middle - half_height);
            outline[2 * COLUMNS - 1 - i] = c.pos(x + dx, middle + half_height);
        }
        let mut mesh = feathered_outline(c, &outline, radius, Color32::WHITE);
        // Join matching top/bottom samples: the inside of a curved lid can be concave.
        for i in 0..COLUMNS as u32 - 1 {
            let top = i * 2;
            let bottom = (2 * COLUMNS as u32 - 1 - i) * 2;
            mesh.add_triangle(top, top + 2, bottom);
            mesh.add_triangle(top + 2, bottom - 2, bottom);
        }
        Shape::mesh(mesh)
    }
}

// A single filled silhouette and one exterior feather retain the same opacity along the
// straight edges and round caps, including during fractional DPI scaling and skin changes.
fn feathered_outline(c: &Card, outline: &[Pos2], radius: f32, color: Color32) -> Mesh {
    let mut mesh = Mesh::default();
    let count = outline.len();
    mesh.vertices.reserve(2 * count + 1);
    let feather = (0.5 / c.ui.ctx().pixels_per_point()).min(radius * c.s * 0.8);
    for (i, &p) in outline.iter().enumerate() {
        let previous = outline[(i + count - 1) % count];
        let next = outline[(i + 1) % count];
        let tangent = ((p - previous).normalized() + (next - p).normalized()).normalized();
        let edge = Vec2::new(tangent.y, -tangent.x) * feather;
        mesh.colored_vertex(p - edge, color);
        mesh.colored_vertex(p + edge, Color32::TRANSPARENT);
        let a = (i * 2) as u32;
        let b = (((i + 1) % count) * 2) as u32;
        mesh.add_triangle(a, a + 1, b);
        mesh.add_triangle(a + 1, b + 1, b);
    }
    mesh
}

fn badge(c: &Card, x: f32, y: f32, radius: f32, color: Color32, heart: bool) {
    if radius < 0.12 { return; }
    // Both outlines are star-shaped about their centre, so a triangle fan fills the notches.
    let outline: [Pos2; 32] = std::array::from_fn(|i| {
        let a = TAU * i as f32 / 32.0;
        let (sin, cos) = a.sin_cos();
        let (dx, dy) = if heart {
            (sin.powi(3), -(13.0 * cos - 5.0 * (2.0*a).cos() - 2.0 * (3.0*a).cos() - (4.0*a).cos()) / 16.0)
        } else {
            (cos.powi(3), sin.powi(3))
        };
        c.pos(x + radius * dx, y + radius * dy)
    });
    let mut mesh = feathered_outline(c, &outline, radius * 0.5, color);
    let center = mesh.vertices.len() as u32;
    mesh.colored_vertex(c.pos(x, y), color);
    for i in 0..outline.len() { mesh.add_triangle(center, (i * 2) as u32, ((i + 1) % outline.len() * 2) as u32); }
    c.painter.add(Shape::mesh(mesh));
}

pub(super) fn draw(c: &Card, cx: f32, expression: Expression, phase: f32) {
    use Expression::*;
    let envelope = (PI * phase).sin().max(0.0).powi(2);
    let turn = (TAU * phase).sin();
    let beat = (2.0 * TAU * phase).sin();
    let strength = smoothstep(0.0, 0.35, phase) * (1.0 - smoothstep(0.65, 1.0, phase));
    let mut x = 0.0;
    let mut y = 14.5;
    let mut eyes = [Eye::OPEN; 2];
    let mut smile = strength;
    let mut cheeks = false;
    match expression {
        Wink => {
            eyes[1] = Eye::OPEN.toward(Eye::SMILE, strength);
            y -= envelope * 0.85;
            badge(c, cx + 13.0 + envelope * 0.8, 11.0 - envelope * 4.5, envelope * 3.7, LILAC, false);
        }
        Smug => {
            eyes[0] = Eye::OPEN.toward(Eye::SLANT, strength);
            eyes[1] = Eye::OPEN.toward(Eye::SMILE, strength);
            x -= envelope * 0.85;
            y -= envelope * 1.2;
            badge(c, cx - 13.0, 10.0 - envelope * 2.6, envelope * 3.3, GOLD, false);
        }
        Blush | ShyPeek => {
            cheeks = true;
            eyes[0] = Eye::OPEN.toward(Eye::SOFT, strength);
            eyes[1] = eyes[0];
            if expression == ShyPeek {
                let peek = smoothstep(0.26, 0.53, phase) * (1.0 - smoothstep(0.56, 0.84, phase));
                eyes[0] = eyes[0].toward(Eye { h: 2.8, ..Eye::OPEN }, peek);
                x -= 1.35 * peek;
            }
            y += envelope * 1.2;
        }
        Delighted | Praised | Heart | Giggle => {
            eyes = [Eye::OPEN.toward(Eye::SMILE, strength); 2];
            y -= envelope * (1.3 + if expression == Giggle { 0.95 * (3.0 * TAU * phase).sin() } else { 0.4 * beat });
            cheeks = expression == Delighted;
            match expression {
                Praised => {
                    for side in [-1.0, 1.0] {
                        let pulse = envelope * (0.85 + 0.15 * beat * side);
                        badge(c, cx + side * (11.0 + 1.5 * envelope), 10.0 - 2.5 * pulse, pulse * 4.1, GOLD, false);
                    }
                }
                Heart => badge(c, cx + 9.0 + 2.2 * envelope, 23.0 - 3.4 * envelope, envelope * (3.2 + 0.45 * beat), PINK, true),
                _ => {}
            }
        }
        SideGlance => {
            x = envelope * 1.5;
            eyes[0] = Eye::OPEN.toward(Eye::SLANT, strength);
            eyes[1].y -= envelope * 1.2;
            eyes[1].h += envelope * 0.6;
        }
        Scan | Peek | Search => {
            smile = 0.0;
            let sweep = turn * (PI * phase).sin();
            match expression {
                Scan => { x = -8.5 * sweep; y += 0.65 * turn * envelope; }
                Peek => {
                    x = -6.6 * sweep;
                    y += envelope * (1.2 + turn);
                    let squint = smoothstep(0.22, 0.46, phase) * (1.0 - smoothstep(0.53, 0.83, phase));
                    eyes = [Eye::OPEN.toward(Eye::SMILE, squint); 2];
                }
                Search => {
                    x = 8.2 * sweep;
                    y += 1.4 * beat * envelope;
                    for (i, eye) in eyes.iter_mut().enumerate() { eye.h += sweep * (i as f32 * 2.0 - 1.0) * 1.15; }
                }
                _ => unreachable!(),
            }
        }
    }
    let cx = cx + x;
    c.painter.add(Shape::Vec(vec![eyes[0].shape(c, cx - 5.6, y), eyes[1].shape(c, cx + 5.6, y)]));
    if cheeks {
        for side in [-1.0, 1.0] {
            c.painter.add(Shape::ellipse_filled(c.pos(cx + 8.7 * side, y + 4.2),
                Vec2::new(1.85, 1.0) * envelope * c.s, PINK.gamma_multiply(envelope)));
        }
    }
    if matches!(expression, Giggle | SideGlance) && envelope > 0.01 {
        for side in [-1.0, 1.0] {
            if expression == SideGlance && side < 0.0 { continue; }
            for offset in [0.0, 2.4] {
                let x = cx + side * (8.8 + offset);
                c.seg((x - 0.7, y + 5.8), (x + 0.7, y + 3.9), 0.9, PINK.gamma_multiply(envelope));
            }
        }
    }
    if smile > 0.01 {
        let width = smile * if expression == Giggle { 3.1 } else { 2.25 };
        let points: [_; 11] = std::array::from_fn(|i| {
            let q = i as f32 / 5.0 - 1.0;
            (cx + q * width, y + 7.1 + (1.0 - q*q) * smile * (1.3 + 0.24 * beat))
        });
        c.path(&points, 1.15, Color32::WHITE.gamma_multiply(smile));
    }
}
