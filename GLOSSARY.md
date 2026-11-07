# Gestion des flakes

Outils de gestion des flakes Nix de la machine : repérer les dépôts flake, choisir ceux qu'on maintient, les mettre à jour et appliquer la configuration système. Le dépôt `nixos-flakes` est un recueil d'outils ; la gestion des flakes n'en est qu'un.

## Language

**Outil**:
Programme autonome du dépôt `nixos-flakes`, packagé et exposé par le flake (la gestion des flakes en est un).
_Avoid_: Script, utilitaire, paquet

**Dépôt flake**:
Dossier contenant un `flake.nix`, qu'il soit sous git ou non.
_Avoid_: Projet, repo (seul), flake (seul)

**Dépôt détecté**:
Dépôt flake trouvé par le scan des racines, mais pas encore choisi par l'utilisateur.
_Avoid_: Dépôt connu, dépôt trouvé

**Dépôt suivi**:
Dépôt flake que l'utilisateur a explicitement choisi de maintenir ; seuls les dépôts suivis sont mis à jour.
_Avoid_: Dépôt actif, dépôt géré

**Dépôt système**:
Dépôt suivi qui décrit la configuration NixOS de la machine ; c'est le seul dont la mise à jour peut être appliquée au système.
_Avoid_: Config machine, dépôt principal

**Racine de scan**:
Dossier à partir duquel on cherche des dépôts flake.
_Avoid_: Dossier de recherche

**Mise à jour**:
Opération qui fait avancer les verrous (`flake.lock`) d'un dépôt flake.
_Avoid_: Upgrade, bump

**Application**:
Opération qui active la configuration d'un dépôt système sur la machine (`nixos-rebuild switch`).
_Avoid_: Rebuild, déploiement

**Mise à niveau automatique**:
Mise à jour suivie d'une application du dépôt système, lancée périodiquement et avec retour arrière en cas d'échec.
_Avoid_: Auto-upgrade
