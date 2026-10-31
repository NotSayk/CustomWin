# Architecture

Principe directeur : **ce qui tourne en permanence ne doit presque rien coûter**. L'interface n'est ouverte que quand on s'en sert ; les futurs composants résidents (fonds animés, widgets) sont en Rust natif, sans WebView.

```
┌──────────────────────────┐    UAC, une fois par opération    ┌───────────────────────────┐
│ customwin-app (Tauri)    │ ────────────────────────────────▶ │ customwin-helper.exe      │
│ interface, non élevée    │   requête JSON (tweaks par id)    │ élevé, catalogue seulement│
│ tweaks HKCU en direct    │ ◀──────────────────────────────── │ HKLM, services, restauration│
└────────────┬─────────────┘          réponse JSON             └─────────────┬─────────────┘
             │                                                               │
             └──────────────────────┬────────────────────────────────────────┘
                                    ▼
                    customwin-core (bibliothèque Rust)
       catalogue · moteur · journal · analyse · backend Windows
```

`customwin-desktop.exe` (phase 2) affiche le fond animé ; les widgets et le mode sombre selon l'heure s'y grefferont.

## customwin-core

| Module | Rôle |
|---|---|
| `model` | Types du catalogue : `Tweak`, `Action` (registre, service, plan d'alimentation, `SystemParametersInfo`, appli AppX, carte réseau, réglage NVIDIA), `Profile` |
| `catalog` | Catalogue TOML intégré au binaire, validé au chargement |
| `engine` | `state` / `apply` / `revert` / `run` |
| `journal` | État d'origine de chaque réglage modifié, écrit de façon atomique |
| `analysis` | Modèle `SystemInfo`, constats (`insights`), recommandations (`recommend`). Code pur, testé sur tout OS |
| `drivers` | Lecture des réponses NVIDIA (catalogue des GPU, dernier pilote) et comparaison de versions. Code pur |
| `backend` | Trait `SystemBackend` et `MockBackend` (en mémoire, insensible à la casse comme Windows) |
| `windows` | Backend réel (`windows-rs`), sondes matérielles (DXGI, registre, IP Helper), élévation, point de restauration, NVAPI (`nvapi.rs`), applis (`appx.rs`), HTTPS (`online.rs`) |

### Garanties du moteur

- **Capture avant écriture** : l'état d'origine de toutes les actions est lu, puis l'entrée du journal est écrite sur disque *avant* la première modification. Un plantage au milieu d'un tweak laisse de quoi l'annuler.
- **Tout ou rien** : si une action échoue, celles déjà faites sont restaurées.
- **Réappliquer ne perd rien** : un tweak déjà journalisé garde son état d'origine.
- **Clés créées** : le journal retient la clé la plus haute que CustomWin a créée ; à l'annulation, les clés vides sont supprimées de la plus profonde à celle-ci, et on s'arrête dès qu'une clé contient autre chose.
- **Valeurs exotiques** : une valeur d'origine de type binaire ou chaîne extensible est conservée octet par octet (`RegValue::Raw`).
- **Actions à cibles multiples** : une action « carte réseau » vise toutes les cartes physiques du bon type (bit `NCF_PHYSICAL`, `*IfType`) ; le journal retient la valeur d'origine de chaque instance.
- **NVIDIA** : le journal distingue une valeur choisie par l'utilisateur (restaurée telle quelle) du réglage par défaut du pilote (restauré en supprimant la valeur du profil).
- **Réversibilité partielle** : une appli supprimée ne revient qu'en la réinstallant depuis le Store. Ces tweaks sont signalés, classés après les autres dans les recommandations, et interdits dans les profils.
- **Journal non fiable** : il est dans un dossier modifiable par l'utilisateur et rejoué par un processus élevé. Une entrée n'est rejouée que si chacune de ses cibles correspond exactement à une action déclarée par le tweak du catalogue (`engine::validate_entry`). Un journal falsifié ne peut donc pas écrire ailleurs.

## Élévation

L'interface tourne toujours sans droits admin, pour qu'une relance de l'Explorateur, et plus tard les fonds animés, restent dans la session normale. Quand une opération touche HKLM ou un service :

1. l'app écrit `%LOCALAPPDATA%\CustomWin\requests\<horodatage>.json` ;
2. elle lance `customwin-helper.exe --request <fichier>` avec le verbe `runas` (invite UAC) et attend la fin ;
3. le helper refuse tout chemin de journal autre que `…\CustomWin\journal.json`, crée le point de restauration, exécute, et écrit `<fichier>.response.json`.

Le helper n'accepte que des identifiants de tweaks du catalogue intégré : aucune commande ni valeur libre.

Élévation « par-dessus l'épaule » (compte standard + identifiants d'un autre administrateur) : le helper verrait le HKCU de l'autre compte. L'app joint donc son SID à la requête ; si celui du helper diffère, toute opération qui touche des réglages par utilisateur est refusée avec une explication. Les réglages machine (services, HKLM, NVIDIA, réseau) restent possibles.

## Accès réseau

CustomWin ne se connecte à internet que sur un clic explicite (« Vérifier les mises à jour en ligne ») :

1. `www.nvidia.com/Download/API/lookupValueSearch.aspx?TypeID=3` : liste des GPU GeForce ;
2. `gfwsl.geforce.com/…/AjaxDriverService.php?func=DriverManualLookup&psid=…&pfid=…` : dernier pilote Game Ready pour ce GPU.

Seul le modèle du GPU est transmis (sous forme d'identifiants produit). Les requêtes passent par WinHTTP, avec le proxy et les certificats du système. Le lien « Voir sur nvidia.com » n'ouvre que des adresses `https://www.nvidia.com/`. La réinstallation d'une appli via winget utilise aussi internet, mais seulement quand l'utilisateur annule ce tweak.

## Fond d'écran animé (`crates/desktop`)

Processus natif séparé, lancé par l'app (ou à l'ouverture de session) et qui s'arrête de lui-même quand il n'y a rien à jouer. L'app écrit `%LOCALAPPDATA%\CustomWin\wallpaper.json` puis lui envoie `WM_APP+1` pour qu'il le relise, ou `WM_CLOSE` pour l'arrêter.

- **Placement** : le message `0x052C` envoyé à `Progman` fait créer à l'Explorateur une couche entre l'image de fond et les icônes. Avant 24H2, c'est un `WorkerW` de premier niveau dont nos fenêtres deviennent les enfants. À partir de 24H2, c'est un enfant de `Progman` : nos fenêtres deviennent enfants de `Progman`, placées juste sous `SHELLDLL_DefView` (les icônes). Une fenêtre par écran ; un écran sans vidéo garde son fond normal.
- **Rendu** : un moteur Media Foundation (`IMFMediaEngine`, mode « frame server ») par fichier vidéo, branché sur le périphérique D3D11 via `IMFDXGIDeviceManager` : le décodage passe par le GPU. À chaque rafraîchissement d'écran, la boucle demande s'il y a une nouvelle image (`OnVideoStreamTick`) et la copie de GPU à GPU dans la swap chain DirectComposition de chaque écran concerné (`TransferVideoFrame`), avec le cadrage calculé par `customwin_core::wallpaper::frame`. Une même vidéo sur deux écrans n'est décodée qu'une fois. Sans DirectComposition (Wine), repli sur des swap chains classiques liées aux fenêtres.
- **Pause** : la règle (`wallpaper::should_play`, testée) combine la fenêtre au premier plan (hooks `EVENT_SYSTEM_FOREGROUND`, minimisation, déplacements de la fenêtre active), le verrouillage de session, l'état de l'écran et la batterie. Un écran en pause garde sa dernière image ; quand plus rien ne joue, les moteurs sont mis en pause et la boucle dort dans `WaitMessage`.
- **Robustesse** : fenêtre de contrôle cachée de premier niveau pour recevoir `TaskbarCreated` (Explorateur relancé) et `WM_DISPLAYCHANGE` ; nouvel essai toutes les 2 s si l'Explorateur n'est pas prêt à l'ouverture de session ; une seule instance (mutex) ; l'image de fond normale est redessinée à l'arrêt.

Vérifié sous Wine (écran virtuel) : décodage H.264, copie GPU, cadrage et boucle tournent ; le placement derrière les icônes et DirectComposition n'existent pas sous Wine et restent à valider sur Windows.

## Interface

Svelte 5 (runes), sans bibliothèque de composants : environ 66 Ko de JS minifié. `src/lib/api.ts` appelle les commandes Tauri ; en dehors de Tauri (`npm run dev` dans un navigateur), un mock prend le relais avec le vrai catalogue exporté par `customwin-helper catalog-json`. Le mock est exclu du build de production.

## Tests

- `cargo test` : moteur, catalogue, journal et analyse contre `MockBackend` (tout OS).
- `crates/core/tests/windows_backend.rs` : backend réel contre le registre, dans une clé jetable `HKCU\Software\CustomWinTest`. Tourne sous Windows en CI, et sous Wine en local.
- CI GitHub Actions (`windows-latest`) : format, tests, clippy `-D warnings`, `svelte-check`, puis génération de l'installeur NSIS en artefact.
