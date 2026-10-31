# Feuille de route

Décisions prises : Windows 11 23H2 et plus récent · Rust + Tauri 2 + Svelte 5 · open source (MIT) · NVIDIA d'abord, AMD et Intel ensuite · tweaks risqués en mode expert uniquement · internet optionnel · interface FR + EN.

## Phase 1 — Tweaks et analyse ✅ (en cours de finition)

- [x] Moteur réversible : journal, tout ou rien, nettoyage des clés créées, validation du journal
- [x] Catalogue de 31 tweaks (confidentialité, pubs, interface, Explorateur, jeu, alimentation, Windows Update, expert)
- [x] Profils Minimal, Gaming, Confidentialité ; « Tout annuler »
- [x] Analyse : OS, CPU, RAM, GPU et version du pilote NVIDIA, plan d'alimentation, batterie, réseau
- [x] Constats : pilote ancien, Ethernet à 100 Mb/s, économie d'énergie sur PC fixe, Wi-Fi seul, enregistrement en arrière-plan, RAM faible
- [x] Questionnaire et recommandations
- [x] Helper élevé + point de restauration
- [ ] Applis préinstallées (bloatware) : suppression via AppX, marquée « partiellement réversible » (réinstallation via winget ou le Store)
- [ ] NVIDIA via NVAPI (MIT) : mode faible latence, gestion de l'alimentation, lecture des profils du pilote
- [ ] Paramètres avancés des cartes réseau (économie d'énergie, interruptions), avec preuves à l'appui
- [ ] Vérification en ligne (optionnelle) de la dernière version du pilote NVIDIA
- [ ] Signature du code (Azure Trusted Signing) pour éviter les alertes SmartScreen et antivirus

## Phase 2 — Profils de contexte

- `customwin-agent.exe` : Rust pur, icône dans la barre des tâches, ~3-5 Mo de RAM
- Déclencheurs : heure, lancement d'un processus (jeu), secteur ou batterie, raccourci clavier
- Actions : plan d'alimentation, fermeture d'applis, arrêt de services, notifications coupées, restauration à la sortie du profil
- Point à surveiller : Windows 11 n'a pas d'API publique pour « Ne pas déranger ». Il faudra une méthode non documentée, isolée et désactivable

## Phase 3 — Thèmes, fonds animés et widgets

Un seul processus natif, `customwin-desktop.exe`, partage le rendu Direct3D 11 / Direct2D :

- **Personnalisation** : fond d'écran (`IDesktopWallpaper`), couleur d'accent, curseurs, icônes du shell, mode sombre selon l'heure
- **Fonds vidéo** (mp4/webm) décodés par le GPU (Media Foundation), un par écran ou étiré sur tous ; gestion de la fenêtre derrière les icônes avant et après 24H2
- **Widgets** : horloge, météo (Open-Meteo, sans clé API), CPU/RAM, musique en cours (SMTC) avec contrôles, raccourcis ; posés sur le bureau ou dans une barre fine (AppBar)
- **Format de skin ouvert** en TOML pour créer et partager des widgets
- **Plein écran** : tout se masque et se met en pause (0 % CPU) dès qu'un jeu ou une appli passe en plein écran, ainsi que sur batterie si l'utilisateur le souhaite
