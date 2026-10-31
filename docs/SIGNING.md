# Signature du code

Un exécutable non signé qui modifie le registre et les services est souvent bloqué par SmartScreen ou signalé par des antivirus. Signer `CustomWin.exe`, `customwin-helper.exe` et l'installeur règle l'essentiel du problème. Cette étape demande un compte du propriétaire du projet : elle n'est pas automatisée tant que ce compte n'existe pas.

## Option recommandée : Azure Trusted Signing

Environ 10 $/mois, sans clé matérielle à gérer.

1. Créer un compte Azure, puis une ressource **Trusted Signing** (le compte doit pouvoir valider une identité : particulier ou entreprise).
2. Faire valider l'identité, puis créer un **profil de certificat** de type *Public Trust*.
3. Créer une *app registration* (principal de service) avec le rôle **Trusted Signing Certificate Profile Signer** sur le compte.
4. Ajouter dans les secrets GitHub du dépôt : `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`, ainsi que le point de terminaison, le nom du compte et celui du profil.
5. Brancher la signature dans Tauri via `bundle.windows.signCommand` (voir la [documentation Tauri sur la signature Windows](https://v2.tauri.app/distribute/sign/windows/)), pour que l'app, le helper (sidecar) et l'installeur soient signés pendant `tauri build`.

## Alternative gratuite pour les projets open source

[SignPath Foundation](https://signpath.org) signe gratuitement les projets open source qui remplissent ses conditions (licence OSI, builds reproductibles en CI publique).

## Après la signature

La réputation SmartScreen se construit avec le nombre de téléchargements : les premières versions peuvent encore afficher un avertissement, qui disparaît avec le temps.
