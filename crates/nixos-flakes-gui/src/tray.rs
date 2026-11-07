//! Icône de notification (StatusNotifierItem) via `ksni`, dans un thread d'arrière-plan.
//!
//! Les événements du menu sont transmis à l'UI par un canal ; la réception est
//! exposée comme une `Subscription` iced (voir `app.rs`).

use crate::model::Overall;
use iced::futures::channel::mpsc::{self, UnboundedReceiver, UnboundedSender};
use ksni::blocking::{Handle, TrayMethods};
use std::sync::{Arc, Mutex, OnceLock};

/// Événement envoyé par le menu de l'icône.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    Open,
    CheckNow,
    Quit,
}

/// État affiché par l'icône.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayState {
    UpToDate,
    Updates(usize),
    Error,
}

impl From<Overall> for TrayState {
    fn from(o: Overall) -> Self {
        match o {
            Overall::Unknown | Overall::UpToDate => TrayState::UpToDate,
            Overall::Updates(n) => TrayState::Updates(n),
            Overall::Error => TrayState::Error,
        }
    }
}

impl TrayState {
    /// Nom d'icône du thème freedesktop.
    pub fn icon_name(self) -> &'static str {
        match self {
            TrayState::UpToDate => "emblem-ok-symbolic",
            TrayState::Updates(_) => "software-update-available-symbolic",
            TrayState::Error => "dialog-error-symbolic",
        }
    }

    /// Couleur (RGB) de la pastille de repli.
    fn color(self) -> [u8; 3] {
        match self {
            TrayState::UpToDate => [0x2e, 0xa0, 0x43],
            TrayState::Updates(_) => [0xe0, 0x8a, 0x1e],
            TrayState::Error => [0xd1, 0x34, 0x38],
        }
    }

    /// Pastilles ARGB32 de repli, pour les thèmes sans icônes `-symbolic`.
    pub fn pixmaps(self) -> Vec<ksni::Icon> {
        [22, 32].into_iter().map(|n| pixmap(self, n)).collect()
    }

    pub fn tooltip(self) -> String {
        match self {
            TrayState::UpToDate => "Tout est à jour".into(),
            TrayState::Updates(1) => "1 mise à jour disponible".into(),
            TrayState::Updates(n) => format!("{n} mises à jour disponibles"),
            TrayState::Error => "Erreur lors de la dernière vérification".into(),
        }
    }
}

/// Disque plein de `size` pixels, bord lissé, au format ARGB32 (ordre réseau).
pub fn pixmap(state: TrayState, size: i32) -> ksni::Icon {
    let [r, g, b] = state.color();
    let n = usize::try_from(size.max(1)).unwrap_or(1);
    let c = n as f32 / 2.0;
    let radius = c - 1.0;
    let mut data = Vec::with_capacity(n * n * 4);
    for y in 0..n {
        for x in 0..n {
            let dx = x as f32 + 0.5 - c;
            let dy = y as f32 + 0.5 - c;
            let d = (dx * dx + dy * dy).sqrt();
            let a = (radius + 0.5 - d).clamp(0.0, 1.0);
            data.extend_from_slice(&[(a * 255.0).round() as u8, r, g, b]);
        }
    }
    ksni::Icon {
        width: size,
        height: size,
        data,
    }
}

/// Résultat de l'enregistrement de l'icône, affiché dans les réglages.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum TrayStatus {
    #[default]
    Disabled,
    Registered,
    Failed(String),
}

impl TrayStatus {
    /// Libellé d'état pour les réglages.
    pub fn text(&self) -> String {
        match self {
            TrayStatus::Disabled => "désactivée.".into(),
            TrayStatus::Registered => {
                "icône enregistrée sur le bus D-Bus (StatusNotifierItem).".into()
            }
            TrayStatus::Failed(e) => format!("échec de l'enregistrement : {e}"),
        }
    }

    /// Conseil affiché quand l'icône est active ou en échec.
    pub fn hint(&self) -> Option<&'static str> {
        match self {
            TrayStatus::Disabled => None,
            TrayStatus::Registered => Some(
                "Si rien n'apparaît : il faut un hôte StatusNotifier (ex. module tray de waybar) ; \
                 sous niri, lancer via niri-session ou spawn-at-startup.",
            ),
            TrayStatus::Failed(_) => Some(
                "Nécessite un hôte StatusNotifier (ex. module tray de waybar) ; \
                 sous niri, lancer via niri-session ou spawn-at-startup.",
            ),
        }
    }
}

/// Libellés du menu, dans l'ordre.
pub const MENU_LABELS: [(&str, TrayEvent); 3] = [
    ("Ouvrir", TrayEvent::Open),
    ("Vérifier maintenant", TrayEvent::CheckNow),
    ("Quitter", TrayEvent::Quit),
];

struct FlakeTray {
    tx: UnboundedSender<TrayEvent>,
    state: TrayState,
}

impl ksni::Tray for FlakeTray {
    fn id(&self) -> String {
        "nixos-flakes-gui".into()
    }

    fn title(&self) -> String {
        "Gestion des flakes".into()
    }

    fn icon_name(&self) -> String {
        self.state.icon_name().into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.state.pixmaps()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            icon_name: self.state.icon_name().into(),
            icon_pixmap: self.state.pixmaps(),
            title: "Gestion des flakes".into(),
            description: self.state.tooltip(),
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.tx.unbounded_send(TrayEvent::Open);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        MENU_LABELS
            .iter()
            .map(|&(label, event)| {
                StandardItem {
                    label: label.into(),
                    activate: Box::new(move |t: &mut FlakeTray| {
                        let _ = t.tx.unbounded_send(event);
                    }),
                    ..Default::default()
                }
                .into()
            })
            .collect()
    }
}

/// Poignée sur l'icône en cours d'exécution.
#[derive(Clone)]
pub struct TrayHandle(Arc<Handle<FlakeTray>>);

impl std::fmt::Debug for TrayHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TrayHandle")
    }
}

impl TrayHandle {
    pub fn set_state(&self, state: TrayState) {
        let _ = self.0.update(|t| t.state = state);
    }

    pub fn shutdown(&self) {
        let _ = self.0.shutdown();
    }
}

/// Récepteur d'événements, pris une fois par la subscription.
static RECEIVER: OnceLock<Mutex<Option<UnboundedReceiver<TrayEvent>>>> = OnceLock::new();
static SENDER: OnceLock<UnboundedSender<TrayEvent>> = OnceLock::new();

fn channel_init() -> &'static UnboundedSender<TrayEvent> {
    SENDER.get_or_init(|| {
        let (tx, rx) = mpsc::unbounded();
        let _ = RECEIVER.set(Mutex::new(Some(rx)));
        tx
    })
}

/// Récepteur du canal (une seule fois ; vide ensuite).
pub fn take_receiver() -> Option<UnboundedReceiver<TrayEvent>> {
    channel_init();
    RECEIVER.get()?.lock().ok()?.take()
}

/// Enregistre l'icône. Bloquant (D-Bus) : à appeler hors du fil de l'UI quand c'est possible.
pub fn spawn(state: TrayState) -> Result<TrayHandle, String> {
    let tx = channel_init().clone();
    FlakeTray { tx, state }
        .spawn()
        .map(|h| TrayHandle(Arc::new(h)))
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_per_state() {
        assert_eq!(TrayState::UpToDate.icon_name(), "emblem-ok-symbolic");
        assert_eq!(
            TrayState::Updates(2).icon_name(),
            "software-update-available-symbolic"
        );
        assert_eq!(TrayState::Error.icon_name(), "dialog-error-symbolic");
    }

    #[test]
    fn overall_mapping() {
        assert_eq!(TrayState::from(Overall::Unknown), TrayState::UpToDate);
        assert_eq!(TrayState::from(Overall::Updates(3)), TrayState::Updates(3));
        assert_eq!(TrayState::from(Overall::Error), TrayState::Error);
    }

    #[test]
    fn tooltips_and_menu() {
        assert_eq!(TrayState::Updates(1).tooltip(), "1 mise à jour disponible");
        assert_eq!(
            TrayState::Updates(4).tooltip(),
            "4 mises à jour disponibles"
        );
        let events: Vec<_> = MENU_LABELS.iter().map(|(_, e)| *e).collect();
        assert_eq!(
            events,
            [TrayEvent::Open, TrayEvent::CheckNow, TrayEvent::Quit]
        );
    }

    #[test]
    fn pixmap_is_argb_disc() {
        let p = pixmap(TrayState::Error, 22);
        assert_eq!((p.width, p.height), (22, 22));
        assert_eq!(p.data.len(), 22 * 22 * 4);
        // Coin transparent, centre opaque et de la couleur d'erreur.
        assert_eq!(p.data[0], 0);
        let mid = (11 * 22 + 11) * 4;
        assert_eq!(&p.data[mid..mid + 4], &[255, 0xd1, 0x34, 0x38]);
        assert_eq!(TrayState::UpToDate.pixmaps().len(), 2);
    }

    #[test]
    fn status_texts() {
        assert_eq!(TrayStatus::default(), TrayStatus::Disabled);
        assert!(TrayStatus::Failed("boom".into()).text().contains("boom"));
        assert!(TrayStatus::Registered
            .hint()
            .unwrap()
            .contains("StatusNotifier"));
        assert!(TrayStatus::Disabled.hint().is_none());
    }

    #[test]
    fn receiver_is_taken_once() {
        assert!(take_receiver().is_some());
        assert!(take_receiver().is_none());
    }
}
