//! Notifications éphémères (« toasts ») : cycle de vie pur, sans iced.
//!
//! Un toast entre (glissement), reste affiché `ttl`, puis sort (glissement inverse)
//! avant d'être retiré. Toutes les fonctions reçoivent l'instant courant.

use crate::anim::ease_out_cubic;
use std::time::{Duration, Instant};

pub const ENTER: Duration = Duration::from_millis(220);
pub const EXIT: Duration = Duration::from_millis(180);
pub const TTL_DEFAULT: Duration = Duration::from_millis(4_000);
pub const TTL_ERROR: Duration = Duration::from_millis(8_000);
/// Nombre maximal de toasts empilés ; les plus anciens disparaissent d'abord.
pub const MAX_VISIBLE: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Success,
    Error,
    Info,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Toast {
    pub id: u64,
    pub kind: ToastKind,
    pub title: String,
    pub body: Option<String>,
    born: Instant,
    ttl: Duration,
    closing_at: Option<Instant>,
}

impl Toast {
    /// Visibilité `[0, 1]` : montée à l'entrée, descente à la sortie.
    pub fn presence(&self, now: Instant) -> f32 {
        let entering = now.saturating_duration_since(self.born).as_secs_f32() / ENTER.as_secs_f32();
        let leaving = self.closing_at.map_or(0.0, |c| {
            now.saturating_duration_since(c).as_secs_f32() / EXIT.as_secs_f32()
        });
        ease_out_cubic(entering) * (1.0 - leaving.clamp(0.0, 1.0))
    }

    fn animating(&self, now: Instant) -> bool {
        let entering = now < self.born + ENTER;
        let leaving = self.closing_at.is_some_and(|c| now < c + EXIT);
        entering || leaving
    }

    #[cfg(test)]
    pub fn is_closing(&self) -> bool {
        self.closing_at.is_some()
    }
}

#[derive(Debug, Default)]
pub struct Toasts {
    items: Vec<Toast>,
    next_id: u64,
}

impl Toasts {
    pub fn items(&self) -> &[Toast] {
        &self.items
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Ajoute un toast ; au-delà de [`MAX_VISIBLE`], le plus ancien est retiré.
    pub fn push(
        &mut self,
        kind: ToastKind,
        title: impl Into<String>,
        body: Option<String>,
        now: Instant,
    ) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let ttl = if kind == ToastKind::Error {
            TTL_ERROR
        } else {
            TTL_DEFAULT
        };
        self.items.push(Toast {
            id,
            kind,
            title: title.into(),
            body,
            born: now,
            ttl,
            closing_at: None,
        });
        while self.items.len() > MAX_VISIBLE {
            self.items.remove(0);
        }
        id
    }

    /// Lance la sortie d'un toast (clic sur fermer).
    pub fn dismiss(&mut self, id: u64, now: Instant) {
        if let Some(t) = self.items.iter_mut().find(|t| t.id == id) {
            t.closing_at.get_or_insert(now);
        }
    }

    /// Avance le cycle : démarre les sorties échues, retire les toasts terminés.
    pub fn tick(&mut self, now: Instant) {
        for t in &mut self.items {
            if t.closing_at.is_none() && now >= t.born + t.ttl {
                t.closing_at = Some(t.born + t.ttl);
            }
        }
        self.items
            .retain(|t| t.closing_at.is_none_or(|c| now < c + EXIT));
    }

    /// Vrai si un toast glisse (entrée ou sortie) : il faut des images fluides.
    pub fn is_animating(&self, now: Instant) -> bool {
        self.items.iter().any(|t| t.animating(now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(t0: Instant, ms: u64) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    #[test]
    fn lifecycle_enter_stay_exit_remove() {
        let t0 = Instant::now();
        let mut ts = Toasts::default();
        let id = ts.push(ToastKind::Success, "ok", None, t0);
        assert_eq!(ts.items().len(), 1);
        assert!(ts.is_animating(t0), "entrée en cours");
        assert_eq!(ts.items()[0].presence(t0), 0.0);

        // Stable : visible, plus d'animation.
        ts.tick(at(t0, 1_000));
        assert!(!ts.is_animating(at(t0, 1_000)));
        assert_eq!(ts.items()[0].presence(at(t0, 1_000)), 1.0);

        // Expiration : sortie animée puis retrait.
        ts.tick(at(t0, 4_050));
        assert!(ts.items()[0].is_closing());
        assert!(ts.is_animating(at(t0, 4_050)));
        assert!(ts.items()[0].presence(at(t0, 4_100)) < 1.0);
        ts.tick(at(t0, 4_000 + 181));
        assert!(ts.is_empty(), "retiré après la sortie (id {id})");
        assert!(!ts.is_animating(at(t0, 5_000)));
    }

    #[test]
    fn errors_stay_longer() {
        let t0 = Instant::now();
        let mut ts = Toasts::default();
        ts.push(ToastKind::Error, "boum", Some("détail".into()), t0);
        ts.tick(at(t0, 5_000));
        assert!(!ts.items()[0].is_closing());
        ts.tick(at(t0, 8_001));
        assert!(ts.items()[0].is_closing());
    }

    #[test]
    fn dismiss_starts_exit_once() {
        let t0 = Instant::now();
        let mut ts = Toasts::default();
        let id = ts.push(ToastKind::Info, "x", None, t0);
        ts.dismiss(id, at(t0, 500));
        ts.dismiss(id, at(t0, 600)); // sans effet
        ts.tick(at(t0, 650));
        assert_eq!(ts.items().len(), 1);
        ts.tick(at(t0, 500 + 181));
        assert!(ts.is_empty());
        ts.dismiss(999, t0); // identifiant inconnu : sans effet
    }

    #[test]
    fn stack_is_capped_dropping_oldest() {
        let t0 = Instant::now();
        let mut ts = Toasts::default();
        for i in 0..(MAX_VISIBLE + 2) {
            ts.push(ToastKind::Info, format!("t{i}"), None, t0);
        }
        assert_eq!(ts.items().len(), MAX_VISIBLE);
        assert_eq!(ts.items()[0].title, "t2");
        let ids: Vec<u64> = ts.items().iter().map(|t| t.id).collect();
        assert!(
            ids.windows(2).all(|w| w[0] < w[1]),
            "ids uniques croissants"
        );
    }

    #[test]
    fn empty_stack_is_idle() {
        let ts = Toasts::default();
        assert!(!ts.is_animating(Instant::now()));
    }
}
