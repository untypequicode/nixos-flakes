//! Animations : courbes d'accélération, interpolations et état de transition.
//!
//! Tout est piloté par un horodatage explicite (`Instant`) passé par l'appelant :
//! aucune horloge globale, donc les comportements (progression, fin, repos) se
//! testent sans attendre. L'application ne demande des images (`window::frames`)
//! que tant qu'une transition est en cours (voir `App::is_animating`).

use iced::Color;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Durées standard des transitions.
pub const FAST: Duration = Duration::from_millis(120);
pub const BASE: Duration = Duration::from_millis(180);
pub const SLOW: Duration = Duration::from_millis(220);

/// Décélération cubique (départ vif, arrivée douce).
pub fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Accélération puis décélération cubique.
pub fn ease_in_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    if t >= 1.0 {
        b
    } else {
        a + (b - a) * t
    }
}

/// Interpolation linéaire de couleurs (composantes RGBA).
pub fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    Color {
        r: lerp(a.r, b.r, t),
        g: lerp(a.g, b.g, t),
        b: lerp(a.b, b.b, t),
        a: lerp(a.a, b.a, t),
    }
}

/// Phase `[0, 1)` d'une animation cyclique de période `period`.
pub fn phase(now: Instant, epoch: Instant, period: Duration) -> f32 {
    let elapsed = now.saturating_duration_since(epoch).as_secs_f32();
    (elapsed / period.as_secs_f32()).fract()
}

/// Pulsation douce `[0, 1]` (0 → 1 → 0) à partir d'une phase.
pub fn pulse(phase: f32) -> f32 {
    0.5 - 0.5 * (phase * std::f32::consts::TAU).cos()
}

/// Transition d'une valeur scalaire vers une cible, avec accélération « ease-out ».
#[derive(Debug, Clone, Copy)]
pub struct Tween {
    from: f32,
    to: f32,
    start: Instant,
    duration: Duration,
}

impl Tween {
    /// Tween au repos sur `value`.
    pub fn new(value: f32, now: Instant) -> Self {
        Tween {
            from: value,
            to: value,
            start: now,
            duration: BASE,
        }
    }

    /// Progression brute `[0, 1]` de la transition en cours.
    pub fn progress(&self, now: Instant) -> f32 {
        if self.duration.is_zero() {
            return 1.0;
        }
        let elapsed = now.saturating_duration_since(self.start).as_secs_f32();
        (elapsed / self.duration.as_secs_f32()).clamp(0.0, 1.0)
    }

    /// Valeur courante.
    pub fn value(&self, now: Instant) -> f32 {
        lerp(self.from, self.to, ease_out_cubic(self.progress(now)))
    }

    pub fn target(&self) -> f32 {
        self.to
    }

    /// Change de cible en repartant de la valeur actuelle (pas de saut visuel).
    /// Sans effet si la cible est déjà celle-ci.
    pub fn retarget(&mut self, to: f32, now: Instant, duration: Duration) {
        if (to - self.to).abs() < f32::EPSILON {
            return;
        }
        self.from = self.value(now);
        self.to = to;
        self.start = now;
        self.duration = duration;
    }

    /// Vrai tant que la valeur n'a pas atteint sa cible.
    pub fn is_animating(&self, now: Instant) -> bool {
        (self.from - self.to).abs() > f32::EPSILON && self.progress(now) < 1.0
    }
}

/// Ensemble de transitions nommées (survol, sélection, groupes repliables…).
///
/// Chaque clé mémorise sa valeur « de repos » (celle qu'on lit quand la clé est
/// absente) : une transition terminée n'est oubliée que si elle y est revenue.
#[derive(Debug, Default)]
pub struct Fades {
    map: HashMap<String, (Tween, f32)>,
}

impl Fades {
    /// Niveau `[0, 1]` de `key`, ou `default` si la clé n'a jamais été touchée.
    pub fn level_or(&self, key: &str, now: Instant, default: f32) -> f32 {
        self.map.get(key).map_or(default, |(t, _)| t.value(now))
    }

    pub fn level(&self, key: &str, now: Instant) -> f32 {
        self.level_or(key, now, 0.0)
    }

    /// Dirige `key` vers `target` (départ et repos à `default` si la clé est nouvelle).
    pub fn set(&mut self, key: &str, target: f32, default: f32, now: Instant, duration: Duration) {
        self.map
            .entry(key.to_string())
            .or_insert_with(|| (Tween::new(default, now), default))
            .0
            .retarget(target, now, duration);
    }

    /// Ramène à 0 toutes les clés dont le nom commence par `prefix`.
    pub fn reset_prefix(&mut self, prefix: &str, now: Instant, duration: Duration) {
        for (k, (t, _)) in &mut self.map {
            if k.starts_with(prefix) {
                t.retarget(0.0, now, duration);
            }
        }
    }

    pub fn any_animating(&self, now: Instant) -> bool {
        self.map.values().any(|(t, _)| t.is_animating(now))
    }

    /// Oublie les transitions terminées et revenues à leur valeur de repos.
    pub fn prune(&mut self, now: Instant) {
        self.map.retain(|_, (t, rest)| {
            t.is_animating(now) || (t.target() - *rest).abs() > f32::EPSILON
        });
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.map.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(t0: Instant, ms: u64) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    #[test]
    fn easing_bounds_and_monotonic() {
        for f in [ease_out_cubic, ease_in_out_cubic] {
            assert_eq!(f(0.0), 0.0);
            assert!((f(1.0) - 1.0).abs() < 1e-6);
            assert_eq!(f(-3.0), 0.0);
            assert!((f(7.0) - 1.0).abs() < 1e-6);
            let mut prev = 0.0;
            for i in 0..=100 {
                let v = f(i as f32 / 100.0);
                assert!(v >= prev - 1e-6, "non monotone en {i}");
                prev = v;
            }
        }
        // ease-out : en avance sur le linéaire.
        assert!(ease_out_cubic(0.5) > 0.5);
    }

    #[test]
    fn lerp_and_colors() {
        assert_eq!(lerp(2.0, 4.0, 0.5), 3.0);
        let c = lerp_color(Color::BLACK, Color::WHITE, 0.5);
        assert!((c.r - 0.5).abs() < 1e-6 && (c.a - 1.0).abs() < 1e-6);
        let t = lerp_color(
            Color::from_rgba(0., 0., 0., 0.),
            Color::from_rgba(0., 0., 0., 1.),
            0.25,
        );
        assert!((t.a - 0.25).abs() < 1e-6);
    }

    #[test]
    fn tween_progress_and_completion() {
        let t0 = Instant::now();
        let mut tw = Tween::new(0.0, t0);
        assert!(!tw.is_animating(t0), "au repos au départ");
        tw.retarget(1.0, t0, Duration::from_millis(200));
        assert!(tw.is_animating(t0));
        assert_eq!(tw.value(t0), 0.0);
        let mid = tw.value(at(t0, 100));
        assert!(mid > 0.5 && mid < 1.0, "ease-out à mi-parcours : {mid}");
        assert!(tw.is_animating(at(t0, 199)));
        assert!(!tw.is_animating(at(t0, 200)));
        assert_eq!(tw.value(at(t0, 5_000)), 1.0);
    }

    #[test]
    fn tween_retarget_continues_from_current_value() {
        let t0 = Instant::now();
        let mut tw = Tween::new(0.0, t0);
        tw.retarget(1.0, t0, Duration::from_millis(200));
        let v = tw.value(at(t0, 60));
        tw.retarget(0.0, at(t0, 60), Duration::from_millis(200));
        assert!((tw.value(at(t0, 60)) - v).abs() < 1e-6, "pas de saut");
        assert_eq!(tw.value(at(t0, 400)), 0.0);
        // Même cible : ne relance pas.
        let before = tw.progress(at(t0, 400));
        tw.retarget(0.0, at(t0, 400), Duration::from_millis(200));
        assert_eq!(tw.progress(at(t0, 400)), before);
    }

    #[test]
    fn tween_before_start_is_clamped() {
        let t0 = Instant::now();
        let later = at(t0, 50);
        let mut tw = Tween::new(0.0, later);
        tw.retarget(1.0, later, FAST);
        assert_eq!(tw.value(t0), 0.0);
    }

    #[test]
    fn phase_and_pulse() {
        let t0 = Instant::now();
        let p = Duration::from_millis(1000);
        assert_eq!(phase(t0, t0, p), 0.0);
        assert!((phase(at(t0, 250), t0, p) - 0.25).abs() < 1e-4);
        assert!(phase(at(t0, 1250), t0, p) < 0.3);
        assert!(pulse(0.0).abs() < 1e-6);
        assert!((pulse(0.5) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn fades_idle_detection_and_prune() {
        let t0 = Instant::now();
        let mut f = Fades::default();
        assert!(!f.any_animating(t0));
        assert_eq!(f.level("a", t0), 0.0);
        assert_eq!(f.level_or("g", t0, 1.0), 1.0);

        f.set("a", 1.0, 0.0, t0, FAST);
        assert!(f.any_animating(t0));
        assert!(f.level("a", at(t0, 60)) > 0.0);
        assert!(!f.any_animating(at(t0, 130)), "repos après la durée");
        assert_eq!(f.level("a", at(t0, 130)), 1.0);

        f.set("a", 0.0, 0.0, at(t0, 200), FAST);
        assert!(f.any_animating(at(t0, 200)));
        f.prune(at(t0, 400));
        assert_eq!(f.len(), 0, "les retours à 0 sont oubliés");
    }

    #[test]
    fn pruning_keeps_states_that_differ_from_rest() {
        let t0 = Instant::now();
        let mut f = Fades::default();
        // Groupe replié : repos = 1 (déplié), cible = 0.
        f.set("grp:A", 0.0, 1.0, t0, FAST);
        f.prune(at(t0, 500));
        assert_eq!(f.len(), 1);
        assert_eq!(f.level_or("grp:A", at(t0, 500), 1.0), 0.0);
        // Revenu au repos : oublié.
        f.set("grp:A", 1.0, 1.0, at(t0, 600), FAST);
        f.prune(at(t0, 900));
        assert_eq!(f.len(), 0);
    }

    #[test]
    fn fades_reset_prefix_only_touches_matching_keys() {
        let t0 = Instant::now();
        let mut f = Fades::default();
        f.set("hov:a", 1.0, 0.0, t0, FAST);
        f.set("sel:a", 1.0, 0.0, t0, FAST);
        f.reset_prefix("hov:", at(t0, 500), FAST);
        assert_eq!(f.level("sel:a", at(t0, 600)), 1.0);
        assert_eq!(f.level("hov:a", at(t0, 700)), 0.0);
    }
}
