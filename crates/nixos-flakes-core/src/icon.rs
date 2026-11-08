//! Icône de l'application : rendu procédural (distance à des segments, bord lissé).
//!
//! Même dessin que `assets/icons/nixos-flakes-gui.svg` : flocon à six branches
//! à chevrons, blanc sur un carré arrondi dégradé. Pour la zone de notification,
//! le flocon est seul, agrandi, et prend la couleur de l'état.

use std::f32::consts::PI;

/// Côté du repère de dessin (celui du `viewBox` du SVG).
const BOX: f32 = 256.0;
const C: f32 = BOX / 2.0;

type Seg = [f32; 4];

/// Segments du flocon (centre au milieu), pour une branche de longueur `l`.
fn flake_segments(l: f32) -> Vec<Seg> {
    let mut segs = Vec::new();
    for k in 0..6 {
        let a = k as f32 * PI / 3.0;
        let rot = |x: f32, y: f32| -> (f32, f32) {
            // Rotation autour du centre ; la branche de référence pointe vers le haut.
            let (s, c) = a.sin_cos();
            (C + x * c - y * s, C + x * s + y * c)
        };
        let (tx, ty) = rot(0.0, -l);
        segs.push([C, C, tx, ty]);
        // (distance au centre, longueur) des deux paires de chevrons.
        for (r, len) in [(0.5 * l, 0.3 * l), (0.78 * l, 0.2 * l)] {
            let (px, py) = rot(0.0, -r);
            let (dx, dy) = (
                len * 50f32.to_radians().sin(),
                len * 50f32.to_radians().cos(),
            );
            for sx in [-1.0, 1.0] {
                let (ex, ey) = rot(sx * dx, -r - dy);
                segs.push([px, py, ex, ey]);
            }
        }
    }
    segs
}

fn seg_dist(px: f32, py: f32, s: &Seg) -> f32 {
    let (vx, vy) = (s[2] - s[0], s[3] - s[1]);
    let (wx, wy) = (px - s[0], py - s[1]);
    let t = ((wx * vx + wy * vy) / (vx * vx + vy * vy)).clamp(0.0, 1.0);
    ((wx - t * vx).powi(2) + (wy - t * vy).powi(2)).sqrt()
}

/// Distance signée au flocon (négative à l'intérieur du trait).
fn flake_sd(px: f32, py: f32, segs: &[Seg], width: f32) -> f32 {
    segs.iter()
        .map(|s| seg_dist(px, py, s))
        .fold(f32::MAX, f32::min)
        - width / 2.0
}

fn round_rect_sd(px: f32, py: f32, half: f32, radius: f32) -> f32 {
    let qx = (px - C).abs() - (half - radius);
    let qy = (py - C).abs() - (half - radius);
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - radius
}

/// Couverture d'un pixel (`px` unités de large) à partir d'une distance signée.
fn cover(sd: f32, px: f32) -> f32 {
    (0.5 - sd / px).clamp(0.0, 1.0)
}

/// Composition « source sur destination » en alpha non prémultiplié.
fn over(dst: [f32; 4], rgb: [f32; 3], a: f32) -> [f32; 4] {
    let out_a = a + dst[3] * (1.0 - a);
    if out_a <= 0.0 {
        return [0.0; 4];
    }
    let mut o = [0.0, 0.0, 0.0, out_a];
    for i in 0..3 {
        o[i] = (rgb[i] * a + dst[i] * dst[3] * (1.0 - a)) / out_a;
    }
    o
}

fn hex(r: u8, g: u8, b: u8) -> [f32; 3] {
    [
        f32::from(r) / 255.0,
        f32::from(g) / 255.0,
        f32::from(b) / 255.0,
    ]
}

fn to_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Rendu RGBA (droit, non prémultiplié) de `size`×`size` pixels, ligne par ligne.
fn render(size: usize, f: impl Fn(f32, f32, f32) -> [f32; 4]) -> Vec<u8> {
    let size = size.max(1);
    let px = BOX / size as f32;
    let mut out = Vec::with_capacity(size * size * 4);
    for y in 0..size {
        for x in 0..size {
            let p = f((x as f32 + 0.5) * px, (y as f32 + 0.5) * px, px);
            out.extend_from_slice(&[to_u8(p[0]), to_u8(p[1]), to_u8(p[2]), to_u8(p[3])]);
        }
    }
    out
}

/// Icône de l'application en RGBA (`size`×`size`).
pub fn app_rgba(size: usize) -> Vec<u8> {
    let segs = flake_segments(84.0);
    let (c0, c1) = (hex(0x52, 0x77, 0xc3), hex(0x7e, 0xba, 0xe4));
    render(size, |x, y, px| {
        let t = (((x - 16.0) + (y - 16.0)) / 448.0).clamp(0.0, 1.0);
        let bg = [
            c0[0] + (c1[0] - c0[0]) * t,
            c0[1] + (c1[1] - c0[1]) * t,
            c0[2] + (c1[2] - c0[2]) * t,
        ];
        let base = over([0.0; 4], bg, cover(round_rect_sd(x, y, 112.0, 52.0), px));
        over(base, [1.0; 3], cover(flake_sd(x, y, &segs, 12.0), px))
    })
}

/// Flocon seul, à la couleur `color` (RGB), en RGBA (`size`×`size`) : icône de notification.
pub fn flake_rgba(color: [u8; 3], size: usize) -> Vec<u8> {
    let segs = flake_segments(120.0);
    let [r, g, b] = color;
    let color = hex(r, g, b);
    render(size, |x, y, px| {
        over([0.0; 4], color, cover(flake_sd(x, y, &segs, 20.0), px))
    })
}

/// Comme `flake_rgba`, en ARGB32 (ordre réseau), attendu par StatusNotifierItem.
pub fn flake_argb(color: [u8; 3], size: usize) -> Vec<u8> {
    let mut data = flake_rgba(color, size);
    for px in data.chunks_exact_mut(4) {
        px.rotate_right(1);
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_buffer_and_pixels() {
        let d = app_rgba(64);
        assert_eq!(d.len(), 64 * 64 * 4);
        let mid = (32 * 64 + 32) * 4;
        assert_eq!(d[mid + 3], 255);
        // Coin hors du carré arrondi : transparent.
        assert_eq!(d[3], 0);
    }

    #[test]
    fn flake_argb_layout() {
        let d = flake_argb([0xd1, 0x34, 0x38], 32);
        assert_eq!(d.len(), 32 * 32 * 4);
        // Centre du flocon : couleur demandée, opaque.
        let mid = (16 * 32 + 16) * 4;
        assert_eq!(&d[mid..mid + 4], &[255, 0xd1, 0x34, 0x38]);
        // Coin haut gauche : transparent.
        assert_eq!(d[0], 0);
    }
}
