//! Lien optionnel avec le démon `nixos-flakes daemon` (D-Bus de session).
//!
//! Le GUI n'a pas besoin du démon : s'il est absent, `events` publie simplement `None`
//! et le GUI continue seul avec ses propres vérifications. Le démon n'a pas non plus
//! besoin du GUI.

use iced::futures::channel::mpsc::Sender;
use iced::futures::{SinkExt, Stream, StreamExt};
use nixos_flakes_core::daemon::{DaemonProxy, DaemonStatus, StateKind};
use std::time::Duration;

/// Délai avant de réessayer la connexion quand le démon est absent.
const RETRY: Duration = Duration::from_secs(5);
/// Relecture périodique : détecte la disparition du démon (aucun signal n'est émis alors).
const POLL: Duration = Duration::from_secs(15);

/// Flux d'états du démon : `Some(état)` à chaque changement, `None` quand il est absent.
pub fn events() -> impl Stream<Item = Option<DaemonStatus>> {
    iced::stream::channel(8, async |mut out: Sender<Option<DaemonStatus>>| loop {
        // `session` ne rend la main que sur échec ou déconnexion.
        let _ = session(&mut out).await;
        if out.send(None).await.is_err() {
            return;
        }
        tokio::time::sleep(RETRY).await;
    })
}

async fn session(out: &mut Sender<Option<DaemonStatus>>) -> Result<(), String> {
    let conn = zbus::Connection::session()
        .await
        .map_err(|e| e.to_string())?;
    let proxy = DaemonProxy::new(&conn).await.map_err(|e| e.to_string())?;
    // Absent => erreur « service inconnu » : on retombe sur le mode seul.
    let mut last = DaemonStatus::from_json(&proxy.get_status().await.map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    out.send(Some(last.clone()))
        .await
        .map_err(|e| e.to_string())?;
    let mut signals = proxy
        .receive_status_changed()
        .await
        .map_err(|e| e.to_string())?;
    let mut poll = tokio::time::interval(POLL);
    poll.tick().await;
    loop {
        let json = tokio::select! {
            sig = signals.next() => {
                let sig = sig.ok_or("flux de signaux terminé")?;
                sig.args().map_err(|e| e.to_string())?.status.to_string()
            }
            _ = poll.tick() => proxy.get_status().await.map_err(|e| e.to_string())?,
        };
        let status = DaemonStatus::from_json(&json).map_err(|e| e.to_string())?;
        if status != last {
            last = status.clone();
            out.send(Some(status)).await.map_err(|e| e.to_string())?;
        }
    }
}

/// Demande au démon une vérification immédiate.
pub async fn check_now() -> Result<(), String> {
    let conn = zbus::Connection::session()
        .await
        .map_err(|e| e.to_string())?;
    DaemonProxy::new(&conn)
        .await
        .map_err(|e| e.to_string())?
        .check_now()
        .await
        .map_err(|e| e.to_string())
}

/// Texte court de l'état du démon, pour la barre latérale.
pub fn label(status: Option<&DaemonStatus>) -> String {
    let Some(s) = status else {
        return "Démon absent".to_string();
    };
    if s.checking {
        return "Vérification en cours…".to_string();
    }
    match s.state {
        StateKind::Unknown => "En attente de vérification".to_string(),
        StateKind::UpToDate => "À jour".to_string(),
        StateKind::Updates if s.updates == 1 => "1 mise à jour disponible".to_string(),
        StateKind::Updates => format!("{} mises à jour disponibles", s.updates),
        StateKind::Error => "Erreur de vérification".to_string(),
    }
}

/// Version courte de [`label`] pour une rangée étroite (le texte complet va en infobulle).
pub fn short_label(status: Option<&DaemonStatus>) -> String {
    match status {
        Some(s) if !s.checking && s.state == StateKind::Updates => {
            if s.updates == 1 {
                "1 mise à jour".to_string()
            } else {
                format!("{} mises à jour", s.updates)
            }
        }
        Some(s) if !s.checking && s.state == StateKind::Unknown => "Pas vérifié".to_string(),
        Some(s) if !s.checking && s.state == StateKind::Error => "Erreur".to_string(),
        Some(s) if s.checking => "Vérification…".to_string(),
        _ => label(status),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(state: StateKind, updates: u32, checking: bool) -> DaemonStatus {
        DaemonStatus {
            state,
            updates,
            checking,
            ..Default::default()
        }
    }

    #[test]
    fn labels() {
        assert_eq!(label(None), "Démon absent");
        assert_eq!(label(Some(&st(StateKind::UpToDate, 0, false))), "À jour");
        assert_eq!(
            label(Some(&st(StateKind::Updates, 1, false))),
            "1 mise à jour disponible"
        );
        assert_eq!(
            label(Some(&st(StateKind::Updates, 3, false))),
            "3 mises à jour disponibles"
        );
        assert_eq!(
            label(Some(&st(StateKind::Updates, 3, true))),
            "Vérification en cours…"
        );
        assert_eq!(
            label(Some(&st(StateKind::Error, 0, false))),
            "Erreur de vérification"
        );
    }
}
