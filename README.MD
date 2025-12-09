# TreeSize Rust (Windows / Linux)

Analyseur d’occupation disque inspiré de **TreeSize** / **WinDirStat**, écrit en **Rust** avec **eframe/egui** pour l’interface graphique, et pensé pour un usage 100 % local.

> Objectif : comprendre rapidement ce qui prend de la place sur un disque (dossier, partition, disque complet) et agir dessus sans passer par un outil tiers ou un service en ligne.

---

## Fonctionnalités

### Scan disque / dossier

- Scan d’un **dossier** ou d’un **lecteur complet** :
  - Sous Windows : `C:\`, `D:\`, etc.
  - Sous Linux : `/`, `$HOME`, etc.
- Scan **multithread** (via `rayon`) pour profiter des CPU multi-cœurs.
- Gestion coopérative de l’annulation :
  - Bouton **« Arrêter »** pour demander l’arrêt du scan.
  - Les threads terminent proprement avant de renvoyer le résultat.

### Vue arborescente (Tree)

- Arborescence de dossiers/fichiers triée par **taille décroissante**.
- Pour chaque entrée :
  - Nom,
  - Taille formatée (Ko / Mo / Go / To),
  - Nombre de fichiers (pour les dossiers),
  - Pourcentage par rapport à la racine scannée.
- Clic gauche : sélectionne l’élément.
- Clic droit : ouvre un **menu contextuel** avec :
  - **Propriétés** (sélection dans le panneau à gauche),
  - **Copier le chemin**,
  - **Copier** / **Couper**,
  - **Coller ici** (dans le dossier courant),
  - **Supprimer…** (avec confirmation).

### Vue Treemap façon WinDirStat

- Représentation graphique des dossiers/fichiers sous forme de **rectangles proportionnels à leur taille**.
- Colorisation déterministe par chemin (même dossier ≈ même teinte).
- Tooltip au survol :
  - Nom,
  - Chemin complet,
  - Taille + pourcentage.
- Clic gauche : sélectionne l’élément (synchro avec le panneau d’infos).
- Clic droit : menu contextuel identique à l’arborescence (Propriétés, Copier chemin, Copier/Couper/Coller, Supprimer…).

### Presse-papier interne (fichiers / dossiers)

- Possibilité de **copier** ou **couper** un fichier/dossier depuis :
  - l’arborescence,
  - la treemap,
  - le panneau latéral.
- **Coller ici** sur un dossier (ou sur un fichier → dans son parent).
- Utilise :
  - `fs::rename` pour le déplacement (avec fallback copie + suppression en cas de cross-device),
  - copie récursive pour les dossiers,
  - gestion des chemins trop longs sous Windows.

### Suppression sécurisée

- Bouton **« Supprimer… »** dans le menu contextuel.
- Fenêtre de confirmation :
  - Affiche le chemin complet,
  - Message d’avertissement en rouge,
  - Bouton Annuler / Supprimer.
- Après suppression, un **rescan automatique** de la racine permet de rafraîchir les résultats.

### Barre de progression du scan

Pour les scans volumineux (disque entier, gros SSD, etc.), le programme affiche une **progression en %** basée sur le volume de données à traiter :

1. **Pré-analyse** (phase rapide) :
   - Parcours du dossier / lecteur choisi,
   - Calcul estimé de la **taille totale** de tous les fichiers (en octets),
   - Cette valeur est utilisée comme référentiel pour la progression.

2. **Scan réel** :
   - Construction de l’arbre des nœuds (`Node`),
   - Pour chaque fichier scanné, le nombre d’octets est ajouté aux **octets scannés**.

3. **Affichage** :
   - Barre de progression + pourcentage dans :
     - la **Top Bar**,
     - le panneau **Actions**.
   - Affichage également des valeurs :
     - `scanné / total` (ex : `120.3 Go / 512.0 Go`).

> Il ne s’agit pas d’une estimation de temps au sens strict, mais d’un bon indicateur de progression sur le **volume de données** traité.

---

## Stack technique

- **Langage** : Rust (édition 2021)
- **UI** : [eframe/egui](https://github.com/emilk/egui)
- **Parallélisation** : [rayon](https://crates.io/crates/rayon)
- **Boîte de dialogue native** : [rfd](https://crates.io/crates/rfd)
- **Gestion de canaux / threads** : [crossbeam-channel](https://crates.io/crates/crossbeam-channel)
- **Gestion des dossiers utilisateurs** : [dirs](https://crates.io/crates/dirs)

Dependencies (extrait de `Cargo.toml`) :

```toml
[dependencies]
eframe = { version = "0.27", features = ["default"] }
rayon = "1.10"
rfd = "0.14"
crossbeam-channel = "0.5"
dirs = "5.0"
```

---

## Installation & exécution

### Prérequis

- **Rust** (toolchain stable) + `cargo`  
  → Installation recommandée via [rustup](https://rustup.rs).

### Récupérer le projet

```bash
git clone <URL_DU_REPO> treesize_rust
cd treesize_rust
```

### Lancer en mode debug

```bash
cargo run
```

### Lancer en mode release (recommandé)

```bash
cargo run --release
```

Le binaire final se trouvera dans :

- `target/release/treesize_rust` (Linux),
- `target\release\treesize_rust.exe` (Windows).

Tu peux ensuite le placer dans un dossier présent dans ton `PATH` si tu veux l’utiliser comme un outil système.

---

## Utilisation

1. **Choisir le mode de scan** (panneau gauche → section *Mode de scan*) :
   - `Dossier` : choisir un dossier via le bouton **« Choisir un dossier… »**.
   - `Lecteur` : sélectionner une racine (ex: `C:\` ou `/`) dans la liste.

2. **Lancer le scan** :
   - Cliquer sur **« Lancer le scan »**.
   - Suivre la progression dans :
     - la top bar (barre + pourcentage),
     - le panneau **Actions**.

3. **Explorer le résultat** :
   - **Arborescence** :
     - Parcourir les dossiers,
     - Cliquer pour sélectionner un élément,
     - Clic droit pour le menu contextuel.
   - **Treemap** :
     - Observer la distribution de l’espace disque,
     - Survol pour voir les infos détaillées,
     - Clic gauche / droit comme pour l’arborescence.

4. **Opérations sur les fichiers/dossiers** :
   - Via le menu contextuel ou le panneau de gauche :
     - Copier / Couper / Coller,
     - Supprimer (avec confirmation),
     - Copier le chemin,
     - Voir les propriétés (taille, nombre de fichiers, chemin complet).

5. **Arrêter un scan en cours** :
   - Bouton **« Arrêter »** dans la section *Actions* :
     - Envoie un signal d’annulation,
     - Les threads de scan se terminent proprement,
     - L’UI reste réactive (redraw régulier).

---

## Limitations actuelles

- Pas encore de :
  - **Filtres avancés** (par taille minimale, extension, etc.),
  - **Top N plus gros fichiers** dans un panneau dédié,
  - **Résumé par extension**,
  - **Comparaison entre deux scans**,
  - **Détection de doublons**.
- Le scan complet d’un gros disque peut prendre du temps (comme avec tout outil de ce type), mais :
  - la progression en % donne un retour immédiat,
  - l’annulation coopérative évite de “bloquer” la session.

---

## Roadmap / idées d’évolution

Quelques pistes d’amélioration possibles :

- **Top 100 plus gros fichiers** :
  - Panneau dédié listant les plus gros fichiers du disque,
  - Clic = focus dans l’arborescence / treemap.

- **Filtres simples** :
  - Taille minimale (ex: n’afficher que > 10 Mo),
  - Pourcentage minimal (ex: n’afficher que > 1 % du total).

- **Règles d’exclusion** :
  - Patterns ignorés lors du scan (`node_modules`, `.git`, etc.),
  - Configuration simple dans l’UI.

- **Résumé par extension** :
  - Tableau `extension -> taille totale / nb fichiers`,
  - Filtre rapide par type de fichier.

- **Comparaison de scans** :
  - Sauvegarde / chargement de snapshots de scan,
  - Diff entre deux états (fichiers ajoutés/supprimés / taille modifiée).

- **Détection de doublons** :
  - Hash des fichiers,
  - Groupes de fichiers identiques, suppression guidée.

---

## Licence MIT

