import type { Lang, Localized } from "./types";

const fr = {
  "nav.analysis": "Analyse",
  "nav.tweaks": "Tweaks",
  "nav.contexts": "Profils de contexte",
  "nav.themes": "Thèmes",
  "nav.widgets": "Widgets",
  "nav.soon": "bientôt",
  "undo.all": "Tout annuler",
  "undo.confirm.title": "Tout annuler ?",
  "undo.confirm.body": "CustomWin va remettre chaque réglage qu'il a modifié dans son état d'origine.",
  "undo.none": "Aucune modification à annuler.",
  cancel: "Annuler",
  confirm: "Confirmer",
  close: "Fermer",

  "analysis.title": "Analyse de ton PC",
  "analysis.subtitle": "Ce que CustomWin a trouvé, et ce qui peut être amélioré.",
  "analysis.rescan": "Relancer l'analyse",
  "analysis.scanning": "Analyse en cours…",
  "analysis.machine": "Ta machine",
  "analysis.findings": "Constats",
  "analysis.no_findings": "Rien d'anormal détecté.",
  "analysis.fix": "Corriger",
  "hw.os": "Système",
  "hw.cpu": "Processeur",
  "hw.ram": "Mémoire",
  "hw.gpu": "Carte graphique",
  "hw.driver": "Pilote",
  "hw.power": "Alimentation",
  "hw.network": "Réseau",
  "hw.laptop": "PC portable",
  "hw.desktop": "PC fixe",
  "hw.on_battery": "sur batterie",
  "hw.none": "aucune connexion",

  "quiz.title": "Qu'est-ce qui compte pour toi ?",
  "quiz.subtitle": "Choisis un ou plusieurs objectifs : la liste des améliorations s'adapte.",
  "goal.gaming": "Jeu",
  "goal.performance": "Réactivité",
  "goal.privacy": "Confidentialité",
  "goal.clean": "Windows épuré",
  "quiz.laptop": "C'est un PC portable",
  "quiz.expert": "Inclure le mode expert",
  "recs.title": "Améliorations proposées",
  "recs.empty": "Tout est déjà réglé pour ces objectifs.",
  "recs.pick_goal": "Choisis au moins un objectif.",
  "recs.apply": "Appliquer {n} améliorations",
  "restore_point": "Créer un point de restauration avant",

  "tweaks.title": "Tweaks Windows",
  "tweaks.subtitle": "Chaque changement est enregistré et s'annule en un clic.",
  "tweaks.profiles": "Profils en un clic",
  "tweaks.apply_profile": "Appliquer",
  "tweaks.profile_done": "Déjà appliqué",
  "tweaks.search": "Rechercher un réglage",
  "tweaks.show_expert": "Afficher le mode expert",
  "tweaks.already": "Déjà réglé ainsi dans Windows",
  "tweaks.unavailable": "Indisponible sur ce PC",
  "tweaks.partial": "Partiellement appliqué",

  "cat.privacy": "Confidentialité",
  "cat.ads": "Publicités",
  "cat.interface": "Interface",
  "cat.explorer": "Explorateur",
  "cat.gaming": "Jeu",
  "cat.power": "Alimentation",
  "cat.updates": "Windows Update",
  "cat.expert": "Mode expert",
  "cat.apps": "Applis préinstallées",
  "cat.network": "Réseau",
  "cat.nvidia": "NVIDIA",
  "badge.partial": "Partiellement réversible",
  "badge.partial.hint": "L'annulation réinstalle l'appli depuis le Microsoft Store (internet requis).",
  "drivers.check": "Vérifier les mises à jour en ligne",
  "drivers.checking": "Vérification…",
  "drivers.uptodate": "Pilote à jour ({v})",
  "drivers.update": "Nouveau pilote disponible : {v}",
  "drivers.open": "Voir sur nvidia.com",
  "drivers.not_found": "GPU introuvable dans le catalogue NVIDIA.",
  "drivers.privacy": "Contacte nvidia.com, rien d'autre n'est envoyé que le modèle du GPU.",

  "badge.admin": "Admin",
  "badge.explorer": "Relance l'Explorateur",
  "badge.sign_out": "Déconnexion requise",
  "badge.reboot": "Redémarrage requis",
  "risk.safe": "Sans risque",
  "risk.low": "Risque faible",
  "risk.medium": "Risque modéré",
  "risk.expert": "Expert",
  "evidence.measured": "Gain mesuré",
  "evidence.situational": "Gain variable",

  "toast.done": "{n} réglages modifiés.",
  "toast.failed": "{n} échecs :",
  "toast.restart_explorer": "Relancer l'Explorateur maintenant",
  "toast.reboot": "Redémarre le PC pour terminer.",
  "toast.sign_out": "Déconnecte-toi puis reconnecte-toi pour terminer.",
  "error.generic": "Erreur :",

  "soon.contexts":
    "Un clic ou un déclencheur automatique (heure, lancement d'un jeu, batterie) pour passer en mode Focus ou Gaming : notifications coupées, applis fermées, plan d'alimentation changé.",
  "soon.themes":
    "Fond d'écran, couleur d'accent, curseurs, icônes, mode sombre selon l'heure, et fonds d'écran vidéo animés qui se mettent en pause pendant les jeux.",
  "soon.widgets":
    "Horloge, météo, CPU/RAM, musique en cours et raccourcis : sur le bureau ou dans une barre fine, avec un format de skin ouvert.",
  "soon.label": "Prévu dans la feuille de route",
} as const;

export type Key = keyof typeof fr;

const en: Record<Key, string> = {
  "nav.analysis": "Analysis",
  "nav.tweaks": "Tweaks",
  "nav.contexts": "Context profiles",
  "nav.themes": "Themes",
  "nav.widgets": "Widgets",
  "nav.soon": "soon",
  "undo.all": "Undo everything",
  "undo.confirm.title": "Undo everything?",
  "undo.confirm.body": "CustomWin will put every setting it changed back to its original state.",
  "undo.none": "Nothing to undo.",
  cancel: "Cancel",
  confirm: "Confirm",
  close: "Close",

  "analysis.title": "Your PC analysis",
  "analysis.subtitle": "What CustomWin found, and what can be improved.",
  "analysis.rescan": "Scan again",
  "analysis.scanning": "Scanning…",
  "analysis.machine": "Your machine",
  "analysis.findings": "Findings",
  "analysis.no_findings": "Nothing unusual found.",
  "analysis.fix": "Fix",
  "hw.os": "System",
  "hw.cpu": "Processor",
  "hw.ram": "Memory",
  "hw.gpu": "Graphics card",
  "hw.driver": "Driver",
  "hw.power": "Power",
  "hw.network": "Network",
  "hw.laptop": "Laptop",
  "hw.desktop": "Desktop",
  "hw.on_battery": "on battery",
  "hw.none": "not connected",

  "quiz.title": "What matters to you?",
  "quiz.subtitle": "Pick one or more goals: the list of improvements adapts.",
  "goal.gaming": "Gaming",
  "goal.performance": "Responsiveness",
  "goal.privacy": "Privacy",
  "goal.clean": "Clean Windows",
  "quiz.laptop": "This is a laptop",
  "quiz.expert": "Include expert mode",
  "recs.title": "Suggested improvements",
  "recs.empty": "Everything is already set for these goals.",
  "recs.pick_goal": "Pick at least one goal.",
  "recs.apply": "Apply {n} improvements",
  restore_point: "Create a restore point first",

  "tweaks.title": "Windows tweaks",
  "tweaks.subtitle": "Every change is recorded and can be undone in one click.",
  "tweaks.profiles": "One-click profiles",
  "tweaks.apply_profile": "Apply",
  "tweaks.profile_done": "Already applied",
  "tweaks.search": "Search settings",
  "tweaks.show_expert": "Show expert mode",
  "tweaks.already": "Already set this way in Windows",
  "tweaks.unavailable": "Not available on this PC",
  "tweaks.partial": "Partially applied",

  "cat.privacy": "Privacy",
  "cat.ads": "Ads",
  "cat.interface": "Interface",
  "cat.explorer": "File Explorer",
  "cat.gaming": "Gaming",
  "cat.power": "Power",
  "cat.updates": "Windows Update",
  "cat.expert": "Expert mode",
  "cat.apps": "Preinstalled apps",
  "cat.network": "Network",
  "cat.nvidia": "NVIDIA",
  "badge.partial": "Partially reversible",
  "badge.partial.hint": "Undoing reinstalls the app from the Microsoft Store (internet required).",
  "drivers.check": "Check for updates online",
  "drivers.checking": "Checking…",
  "drivers.uptodate": "Driver up to date ({v})",
  "drivers.update": "New driver available: {v}",
  "drivers.open": "View on nvidia.com",
  "drivers.not_found": "GPU not found in NVIDIA's catalog.",
  "drivers.privacy": "Contacts nvidia.com; only the GPU model is sent.",

  "badge.admin": "Admin",
  "badge.explorer": "Restarts Explorer",
  "badge.sign_out": "Sign-out required",
  "badge.reboot": "Reboot required",
  "risk.safe": "No risk",
  "risk.low": "Low risk",
  "risk.medium": "Moderate risk",
  "risk.expert": "Expert",
  "evidence.measured": "Measured gain",
  "evidence.situational": "Variable gain",

  "toast.done": "{n} settings changed.",
  "toast.failed": "{n} failed:",
  "toast.restart_explorer": "Restart Explorer now",
  "toast.reboot": "Restart the PC to finish.",
  "toast.sign_out": "Sign out and back in to finish.",
  "error.generic": "Error:",

  "soon.contexts":
    "One click or an automatic trigger (time, game launch, battery) to switch to Focus or Gaming: notifications muted, apps closed, power plan changed.",
  "soon.themes":
    "Wallpaper, accent colour, cursors, icons, dark mode by time of day, and animated video wallpapers that pause during games.",
  "soon.widgets":
    "Clock, weather, CPU/RAM, now playing and shortcuts: on the desktop or in a slim bar, with an open skin format.",
  "soon.label": "Planned on the roadmap",
};

function initialLang(): Lang {
  try {
    const saved = localStorage.getItem("lang");
    if (saved === "fr" || saved === "en") return saved;
  } catch {
    /* storage unavailable */
  }
  return navigator.language.toLowerCase().startsWith("fr") ? "fr" : "en";
}

export const i18n = $state({ lang: initialLang() });

export function setLang(lang: Lang) {
  i18n.lang = lang;
  document.documentElement.lang = lang;
  try {
    localStorage.setItem("lang", lang);
  } catch {
    /* storage unavailable */
  }
}

export function t(key: Key, vars: Record<string, string | number> = {}): string {
  const text: string = (i18n.lang === "fr" ? fr : en)[key];
  return text.replace(/\{(\w+)\}/g, (_, k) => String(vars[k] ?? ""));
}

export function loc(l: Localized): string {
  return l[i18n.lang];
}
