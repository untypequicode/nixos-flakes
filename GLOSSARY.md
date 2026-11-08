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
Opération qui fait avancer les verrous (`flake.lock`) d'un dépôt flake, pour tous ses inputs ou pour une sélection d'inputs.
_Avoid_: Upgrade, bump

**Input**:
Dépendance déclarée dans le `flake.nix` d'un dépôt flake, que l'on peut mettre à jour seule ou avec d'autres.
_Avoid_: Dépendance, dépôt

**Application**:
Opération qui active la configuration d'un dépôt système sur la machine (`nixos-rebuild switch`).
_Avoid_: Rebuild, déploiement

**Mise à niveau automatique**:
Mise à jour suivie d'une application du dépôt système, lancée périodiquement et avec retour arrière en cas d'échec.
_Avoid_: Auto-upgrade

**Jeton GitHub**:
Secret personnel saisi dans l'interface qui authentifie les accès à GitHub pour relever la limite de requêtes ; il ne figure jamais dans la configuration Nix.
_Avoid_: Token, clé API

**Fichier de config**:
Fichier Nix du dépôt système (ou d'un dépôt suivi) que l'on peut ouvrir, soit en code, soit en vue graphique.
_Avoid_: Module (seul), fichier de conf

**Vue graphique**:
Présentation d'un fichier de config sous forme de réglages éditables ; les parties trop complexes y sont signalées comme modifiables en code seulement.
_Avoid_: GUI (seul), mode visuel

**Paquet déclaré**:
Logiciel nommé dans une liste de paquets d'un fichier de config, donc installé par cette configuration.
_Avoid_: Paquet installé, application

**Emplacement**:
Liste de paquets précise d'un fichier de config où un paquet déclaré se trouve ou peut être ajouté.
_Avoid_: Destination, cible

**Zone de notification**:
Zone de la barre où l'outil affiche son icône d'état.
_Avoid_: Tray, systray

**Droits système**:
Élévation de privilèges demandée uniquement pour l'application du système et le retour arrière.
_Avoid_: Sudo, root
