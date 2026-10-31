# Architecture

Principe directeur : **ce qui tourne en permanence ne doit presque rien coûter**. L'interface n'est ouverte que quand on s'en sert ; les futurs composants résidents (agent, fonds animés, widgets) sont en Rust natif, sans WebView.

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

Prévu (phases 2 et 3) : `customwin-agent.exe` (déclencheurs de profils, ~3-5 Mo de RAM) et `customwin-desktop.exe` (fonds vidéo + widgets dessinés en Direct3D/Direct2D).

## customwin-core

| Module | Rôle |
|---|---|
| `model` | Types du catalogue : `Tweak`, `Action` (registre, service, plan d'alimentation, `SystemParametersInfo`), `Profile` |
| `catalog` | Catalogue TOML intégré au binaire, validé au chargement |
| `engine` | `state` / `apply` / `revert` / `run` |
| `journal` | État d'origine de chaque réglage modifié, écrit de façon atomique |
| `analysis` | Modèle `SystemInfo`, constats (`insights`), recommandations (`recommend`). Code pur, testé sur tout OS |
| `backend` | Trait `SystemBackend` et `MockBackend` (en mémoire, insensible à la casse comme Windows) |
| `windows` | Backend réel (`windows-rs`), sondes matérielles (DXGI, registre, IP Helper), élévation, point de restauration |

### Garanties du moteur

- **Capture avant écriture** : l'état d'origine de toutes les actions est lu, puis l'entrée du journal est écrite sur disque *avant* la première modification. Un plantage au milieu d'un tweak laisse de quoi l'annuler.
- **Tout ou rien** : si une action échoue, celles déjà faites sont restaurées.
- **Réappliquer ne perd rien** : un tweak déjà journalisé garde son état d'origine.
- **Clés créées** : le journal retient la clé la plus haute que CustomWin a créée ; à l'annulation, les clés vides sont supprimées de la plus profonde à celle-ci, et on s'arrête dès qu'une clé contient autre chose.
- **Valeurs exotiques** : une valeur d'origine de type binaire ou chaîne extensible est conservée octet par octet (`RegValue::Raw`).
- **Journal non fiable** : il est dans un dossier modifiable par l'utilisateur et rejoué par un processus élevé. Une entrée n'est rejouée que si chacune de ses cibles correspond exactement à une action déclarée par le tweak du catalogue (`engine::validate_entry`). Un journal falsifié ne peut donc pas écrire ailleurs.

## Élévation

L'interface tourne toujours sans droits admin, pour qu'une relance de l'Explorateur, et plus tard les fonds animés, restent dans la session normale. Quand une opération touche HKLM ou un service :

1. l'app écrit `%LOCALAPPDATA%\CustomWin\requests\<horodatage>.json` ;
2. elle lance `customwin-helper.exe --request <fichier>` avec le verbe `runas` (invite UAC) et attend la fin ;
3. le helper refuse tout chemin de journal autre que `…\CustomWin\journal.json`, crée le point de restauration, exécute, et écrit `<fichier>.response.json`.

Le helper n'accepte que des identifiants de tweaks du catalogue intégré : aucune commande ni valeur libre.

Limite connue : avec une élévation « par-dessus l'épaule » (compte standard + identifiants d'un autre administrateur), le helper voit le HKCU de l'administrateur. CustomWin vise d'abord les comptes administrateurs classiques ; ce cas sera détecté plus tard en comparant les SID.

## Interface

Svelte 5 (runes), sans bibliothèque de composants : environ 66 Ko de JS minifié. `src/lib/api.ts` appelle les commandes Tauri ; en dehors de Tauri (`npm run dev` dans un navigateur), un mock prend le relais avec le vrai catalogue exporté par `customwin-helper catalog-json`. Le mock est exclu du build de production.

## Tests

- `cargo test` : moteur, catalogue, journal et analyse contre `MockBackend` (tout OS).
- `crates/core/tests/windows_backend.rs` : backend réel contre le registre, dans une clé jetable `HKCU\Software\CustomWinTest`. Tourne sous Windows en CI, et sous Wine en local.
- CI GitHub Actions (`windows-latest`) : format, tests, clippy `-D warnings`, `svelte-check`, puis génération de l'installeur NSIS en artefact.
