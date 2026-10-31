# Feuille de route

Décisions prises : usage perso (pas de signature du code) · pas de profils de contexte · Windows 11 23H2 et plus récent · Rust + Tauri 2 + Svelte 5 · open source (MIT) · NVIDIA d'abord, AMD et Intel ensuite · tweaks risqués en mode expert uniquement · internet optionnel · interface FR + EN.

## Phase 1 — Tweaks et analyse ✅

- [x] Moteur réversible : journal, tout ou rien, nettoyage des clés créées, validation du journal
- [x] Catalogue de 53 tweaks (confidentialité, pubs, interface, Explorateur, applis préinstallées, jeu, NVIDIA, réseau, alimentation, Windows Update, expert)
- [x] Profils Minimal, Gaming, Confidentialité ; « Tout annuler »
- [x] Analyse : OS, CPU, RAM, GPU et version du pilote NVIDIA, plan d'alimentation, batterie, réseau
- [x] Constats : pilote ancien, Ethernet à 100 Mb/s, économie d'énergie sur PC fixe, Wi-Fi seul, enregistrement en arrière-plan, RAM faible
- [x] Questionnaire et recommandations
- [x] Helper élevé + point de restauration
- [x] Refus des réglages par utilisateur quand l'élévation se fait avec un autre compte (comparaison des SID)
- [x] Applis préinstallées : 17 applis, suppression via `PackageManager`, réinstallation via winget (identifiants Store vérifiés), marquées « partiellement réversible » et exclues des profils
- [x] NVIDIA via NVAPI (SDK MIT) : mode faible latence, performances maximales ; retour à la valeur d'origine ou au réglage par défaut du pilote
- [x] Cartes réseau physiques : Energy-Efficient Ethernet, Green Ethernet (Realtek), mise hors tension par Windows
- [x] Vérification en ligne, sur demande, de la dernière version du pilote NVIDIA (WinHTTP, aucune dépendance ajoutée)

## Phase 2 — Thèmes, fonds animés et widgets

- [x] Fonds vidéo : `customwin-desktop.exe` (355 Ko), décodage GPU via Media Foundation, copie GPU vers des swap chains DirectComposition, un fond par écran ou une vidéo sur tous les écrans, cadrage « remplir » ou « vidéo entière »
- [x] Pause automatique (moteurs de décodage arrêtés, 0 % CPU) : jeu ou appli en plein écran, appli agrandie (option), batterie (option), PC verrouillé, écran éteint
- [x] Bureau Windows 11 avant et après 24H2, reprise après un redémarrage de l'Explorateur ou un changement d'écrans, lancement au démarrage de Windows
- [ ] Couleur d'accent, curseurs, icônes, mode sombre selon l'heure
- [ ] Widgets

Un seul processus natif, `customwin-desktop.exe`, partage le rendu Direct3D 11 / Direct2D :

- **Personnalisation** : fond d'écran (`IDesktopWallpaper`), couleur d'accent, curseurs, icônes du shell, mode sombre selon l'heure
- **Fonds vidéo** (mp4/webm) décodés par le GPU (Media Foundation), un par écran ou étiré sur tous ; gestion de la fenêtre derrière les icônes avant et après 24H2
- **Widgets** : horloge, météo (Open-Meteo, sans clé API), CPU/RAM, musique en cours (SMTC) avec contrôles, raccourcis ; posés sur le bureau ou dans une barre fine (AppBar)
- **Format de skin ouvert** en TOML pour créer et partager des widgets
- **Plein écran** : tout se masque et se met en pause (0 % CPU) dès qu'un jeu ou une appli passe en plein écran, ainsi que sur batterie si l'utilisateur le souhaite

## À corriger au prochain patch

- [x] **Fond animé : impossible de retirer ou changer une vidéo supprimée.** `set_wallpaper` (`app/src-tauri/src/main.rs`) vérifie `is_file()` sur *toutes* les vidéos de la config à chaque enregistrement. Si le fichier d'origine a été supprimé et référencé par plusieurs emplacements (un par écran + « tous les écrans »), retirer ou changer un seul emplacement échoue encore (« not a readable video »), à cause des autres. Correctif : ne valider que les chemins nouveaux ou modifiés par rapport à la config enregistrée, et laisser passer un emplacement qui passe à `null`. Idéalement, `get_wallpaper` signale aussi les vidéos introuvables pour que l'interface les affiche comme manquantes.
- [x] **`wallpaper.json` écrit avec un BOM UTF-8** (cas rencontré en l'éditant depuis PowerShell 5.1) : `WallpaperConfig::load` échoue avec « expected value at line 1 column 1 ». Correctif : ignorer un éventuel BOM à la lecture.
