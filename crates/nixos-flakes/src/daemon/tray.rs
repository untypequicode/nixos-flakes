//! Icône d'état (StatusNotifierItem) hébergée par le démon, via `ksni`.

use super::Shared;
use nixos_flakes_core::daemon::{DaemonStatus, StateKind};
use nixos_flakes_core::icon::flake_argb;
use std::sync::Arc;

/// Tailles des pixmaps proposées à l'hôte de la zone de notification.
const SIZES: [i32; 6] = [16, 22, 24, 32, 48, 64];

/// Couleur (RGB) du flocon selon l'état.
pub fn color(kind: StateKind) -> [u8; 3] {
    match kind {
        StateKind::Unknown | StateKind::UpToDate => [0x2e, 0xa0, 0x43],
        StateKind::Updates => [0xe0, 0x8a, 0x1e],
        StateKind::Error => [0xd1, 0x34, 0x38],
    }
}

/// Phrase d'état affichée dans l'infobulle et en tête du menu.
pub fn describe(status: &DaemonStatus) -> String {
    if status.checking {
        return "Vérification en cours…".into();
    }
    match status.state {
        StateKind::Unknown => "Pas encore vérifié".into(),
        StateKind::UpToDate => "Tout est à jour".into(),
        StateKind::Updates if status.updates == 1 => "1 mise à jour disponible".into(),
        StateKind::Updates => format!("{} mises à jour disponibles", status.updates),
        StateKind::Error => status
            .message
            .as_deref()
            .and_then(|m| m.lines().next())
            .map_or_else(|| "Erreur".into(), |l| format!("Erreur : {l}")),
    }
}

fn pixmaps(kind: StateKind) -> Vec<ksni::Icon> {
    SIZES
        .into_iter()
        .map(|size| ksni::Icon {
            width: size,
            height: size,
            data: flake_argb(color(kind), usize::try_from(size).unwrap_or(1)),
        })
        .collect()
}

/// Icône d'état : pas de « Quitter », le démon vit avec la session.
pub struct StatusTray {
    pub status: DaemonStatus,
    pub shared: Arc<Shared>,
    pub gui_command: Option<String>,
}

/// Lance l'interface graphique (sans l'attendre plus qu'il ne faut).
fn open_gui(command: &str) {
    let Some(parts) = super::cmdline::split(command) else {
        eprintln!("commande de l'interface invalide (guillemet non fermé) : {command}");
        return;
    };
    let mut parts = parts.into_iter();
    let Some(program) = parts.next() else { return };
    match std::process::Command::new(program)
        .args(parts)
        .stdin(std::process::Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            // Récolte le processus fils à sa fin pour ne pas laisser de zombie.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => eprintln!("impossible de lancer « {command} » : {e}"),
    }
}

impl ksni::Tray for StatusTray {
    fn id(&self) -> String {
        "nixos-flakes".into()
    }

    fn title(&self) -> String {
        "Gestion des flakes".into()
    }

    fn icon_name(&self) -> String {
        String::new()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        pixmaps(self.status.state)
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            icon_name: String::new(),
            icon_pixmap: pixmaps(self.status.state),
            title: "Gestion des flakes".into(),
            description: describe(&self.status),
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        if let Some(command) = &self.gui_command {
            open_gui(command);
        }
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        let mut items: Vec<ksni::MenuItem<Self>> = vec![
            StandardItem {
                label: describe(&self.status),
                enabled: false,
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: "Vérifier maintenant".into(),
                activate: Box::new(|t: &mut StatusTray| t.shared.check_now.notify_one()),
                ..Default::default()
            }
            .into(),
        ];
        if let Some(command) = self.gui_command.clone() {
            items.push(
                StandardItem {
                    label: "Ouvrir l'interface".into(),
                    activate: Box::new(move |_: &mut StatusTray| open_gui(&command)),
                    ..Default::default()
                }
                .into(),
            );
        }
        items
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptions() {
        let mut s = DaemonStatus::default();
        assert_eq!(describe(&s), "Pas encore vérifié");
        s.state = StateKind::Updates;
        s.updates = 1;
        assert_eq!(describe(&s), "1 mise à jour disponible");
        s.updates = 3;
        assert_eq!(describe(&s), "3 mises à jour disponibles");
        s.state = StateKind::Error;
        s.message = Some("boom\nsuite".into());
        assert_eq!(describe(&s), "Erreur : boom");
        s.checking = true;
        assert_eq!(describe(&s), "Vérification en cours…");
    }

    #[test]
    fn colors_and_pixmaps() {
        assert_ne!(color(StateKind::UpToDate), color(StateKind::Error));
        assert_eq!(color(StateKind::Unknown), color(StateKind::UpToDate));
        let p = pixmaps(StateKind::Error);
        assert_eq!(p.len(), SIZES.len());
        assert_eq!(p[1].data.len(), 22 * 22 * 4);
    }
}
