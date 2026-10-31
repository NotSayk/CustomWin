# CustomWin

Optimise et personnalise Windows 11 **en un clic, et de façon réversible**.

- **Analyse** de la machine (Windows, processeur, GPU NVIDIA et pilote, alimentation, réseau) avec des constats concrets : pilote ancien, câble Ethernet bridé à 100 Mb/s, mode économie d'énergie sur un PC fixe…
- **Questionnaire** (jeu, réactivité, confidentialité, Windows épuré) qui produit la liste des réglages à améliorer, appliquée en un clic.
- **Tweaks** : télémétrie, pubs, Widgets, menu contextuel classique, barre des tâches, animations, Game DVR, HAGS, **applis préinstallées**, **pilote NVIDIA** (faible latence, performances maximales), **cartes réseau** (Energy-Efficient Ethernet, mise hors tension)… avec des **profils** (Minimal, Gaming, Confidentialité) et un bouton **Tout annuler**.
- **Pilote NVIDIA à jour ?** Vérification en ligne sur demande, auprès de NVIDIA uniquement.
- Chaque tweak indique son **niveau de risque** et ce qu'on sait réellement de son effet (« gain mesuré », « gain variable » ou simplement fonctionnel). Les « optimisations » placebo qui circulent sur internet sont volontairement absentes.

Prévu ensuite : thèmes et fonds d'écran vidéo animés, widgets de bureau ultra-légers. Voir [la feuille de route](docs/ROADMAP.md).

> État : **phase 1 terminée** (moteur de tweaks, analyse, interface). Pas encore de version publiée.

## Sécurité et réversibilité

1. Avant de modifier quoi que ce soit, CustomWin enregistre la valeur d'origine dans un journal (`%LOCALAPPDATA%\CustomWin\journal.json`), écrit sur disque **avant** la première modification.
2. Un tweak est tout ou rien : si une de ses actions échoue, les précédentes sont annulées.
3. « Tout annuler » rejoue le journal à l'envers, et supprime les clés de registre que CustomWin avait créées.
4. Pour les réglages système, un point de restauration Windows est créé avant d'appliquer.
5. Les seuls tweaks qui ne s'annulent pas entièrement sur place (suppression d'applis : l'annulation les réinstalle depuis le Store) sont signalés « partiellement réversible » et jamais inclus dans un profil.
6. L'interface ne tourne jamais en administrateur. Seul `customwin-helper.exe` est élevé (une invite UAC par opération), et il n'exécute que des tweaks du catalogue, jamais de commande arbitraire.

## Installer l'app

L'installeur n'est pas signé (inutile pour un usage perso). Au premier lancement, Windows affiche « Windows a protégé votre ordinateur » : clique sur **Informations complémentaires**, puis **Exécuter quand même**. Certains antivirus peuvent aussi se méfier d'un outil non signé qui modifie le registre.

## Structure

| Dossier | Contenu |
|---|---|
| `crates/core` | Moteur de tweaks, catalogue (`catalog/*.toml`), journal, analyse, backend Windows |
| `crates/helper` | `customwin-helper.exe` : exécutant élevé, aussi utilisable en ligne de commande |
| `app` | Interface Tauri 2 + Svelte 5 |

Détails : [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Développer

Prérequis : Rust stable, Node 22, et sous Windows les [prérequis Tauri](https://v2.tauri.app/start/prerequisites/) (WebView2 est déjà présent sur Windows 11).

```bash
cargo test
```

```bash
cd app && npm install && npx tauri dev
```

Pour travailler sur l'interface depuis n'importe quel OS, avec une machine simulée et le vrai catalogue :

```bash
cd app && npm run mock-catalog && npm run dev
```

Le helper s'utilise aussi seul, depuis un terminal administrateur :

```bash
customwin-helper list
```

```bash
customwin-helper profile gaming
```

```bash
customwin-helper revert-all
```

## Ajouter un tweak

Ajoute une entrée dans `crates/core/catalog/*.toml` :

```toml
[[tweak]]
id = "interface.exemple"
category = "interface"
risk = "safe"            # safe | low | medium | expert
evidence = "functional"  # functional | measured | situational
restart = "explorer"     # none | explorer | sign_out | reboot
tags = ["clean"]         # objectifs : gaming, performance, privacy, clean ; contrainte : desktop-only
name = { fr = "…", en = "…" }
description = { fr = "…", en = "…" }

[[tweak.action]]
kind = "registry"
hive = "HKCU"
path = 'Software\…'
name = "Valeur"
dword = 0
```

Les tests vérifient au chargement que les identifiants sont uniques, les traductions présentes, et qu'aucun tweak « expert » n'est dans un profil.

## Licence

[MIT](LICENSE)

---

**English** — CustomWin is a lightweight, one-click and fully reversible Windows 11 optimizer and customizer (Rust + Tauri). Every change is journaled before it happens and can be undone; only a small helper ever runs elevated. The UI is available in French and English.
