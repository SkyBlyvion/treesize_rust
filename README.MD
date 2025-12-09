# TreeSize Rust (Windows / Linux)

Analyseur d’occupation disque inspiré de TreeSize / WinDirStat, écrit en **Rust** avec **eframe/egui** pour l’interface graphique.

Ce projet est pensé pour un usage **local**, sur **Windows** et **Linux**, sans déploiement ni distribution commerciale.  
Il permet de :

- Scanner un **dossier** ou un **lecteur complet** (C:, D:, /, $HOME, …).
- Visualiser les résultats :
  - sous forme d’**arborescence triée par taille** ;
  - sous forme de **treemap** (type WinDirStat).
- Interagir avec le système de fichiers :
  - **Propriétés** (afficher le chemin, la taille, le nombre de fichiers) ;
  - **Copier le chemin** ;
  - **Copier / Couper / Coller ici** (fichiers et dossiers) ;
  - **Suppression** avec confirmation.
- Gérer des scans lourds :
  - **Scan parallèle** via `rayon` ;
  - **Annulation coopérative** du scan ;
  - Gestion d’erreurs d’accès et des chemins trop longs (Windows).

---

## 1. Fonctionnalités clés

### 1.1 Scan disque

- Deux modes de scan :
  - **Mode Dossier** : sélection d’un dossier via boîte de dialogue native.
  - **Mode Lecteur** :
    - Windows : détection des lecteurs C: à Z: existants.
    - Linux/Unix : racine `/` + répertoire home.
- Scan récursif du système de fichiers :
  - Structure de données `Node` (dossier/fichier) avec :
    - `name`, `path`, `is_dir`
    - `size` (taille cumulée pour les dossiers)
    - `file_count` (nombre total de fichiers descendants)
    - `children` (sous-éléments)

### 1.2 Vue arborescente

- Vue hiérarchique **triée par taille décroissante**.
- Pour chaque élément :
  - Nom + taille formatée (Ko, Mo, Go, To)
  - Pour les dossiers : nombre de fichiers et pourcentage de la racine.
- Interactions (clic sur un élément) :
  - **Clic gauche** : sélectionne le nœud (affichage dans le panneau latéral).
  - **Clic droit** : ouvre un menu contextuel avec :
    - `Propriétés`
    - `Copier le chemin`
    - `Copier` (presse-papier interne)
    - `Couper`
    - `Coller ici` (si un élément est dans le presse-papier)
    - `Supprimer…` (avec fenêtre de confirmation)

### 1.3 Vue Treemap

- Représentation graphique de l’occupation disque type **WinDirStat** :
  - Chaque bloc représente un fichier ou un dossier.
  - **Surface proportionnelle à la taille**.
  - Coloration déterministe à partir du chemin (`color_for_path`).
- Interactions :
  - **Clic gauche** : sélectionne l’élément correspondant.
  - **Survol** : affiche un tooltip détaillé (chemin, taille, pourcentage).
  - **Clic droit** sur un bloc :
    - `Propriétés`
    - `Copier le chemin`
    - `Copier` / `Couper`
    - `Coller ici` :
      - sur un dossier : colle à l’intérieur ;
      - sur un fichier : colle dans le dossier parent du fichier.
    - `Supprimer…` (avec confirmation).

### 1.4 Opérations fichiers (copier / couper / coller / supprimer)

- **Presse-papier interne** :
  - Chemin stocké dans `clipboard_path`.
  - Mode **Copier** ou **Couper** (`clipboard_is_cut`).
- **Coller ici** :
  - Utilise `pending_paste_dest` pour déclencher l’opération après le rendu.
  - Appelle `handle_paste` → `copy_or_move`.
- **Copie / Déplacement** (`copy_or_move`) :
  - Validation des chemins source/destination.
  - Protection contre le déplacement d’un dossier dans lui-même.
  - Gestion des **chemins trop longs** sur Windows (`is_path_too_long`).
  - Déplacement :
    - tentative de `fs::rename` ;
    - fallback **copie récursive + suppression** si nécessaire (cross-device).
- **Suppression** (`delete_path`) :
  - Fenêtre de confirmation dédiée.
  - Dossiers : `remove_dir_all`.
  - Fichiers : `remove_file`.

---

## 2. Architecture technique

### 2.1 Technologies

- Langage : **Rust**
- GUI : **eframe** / **egui**
- Concurrence / parallélisme :
  - **rayon** pour le scan parallèle (`par_iter`).
  - **crossbeam-channel** pour le retour de résultat de scan.
  - **AtomicBool** + `Arc` pour l’annulation de scan.
- Dialogues natifs : **rfd** (`pick_directory`).
- Gestion des répertoires utilisateurs :
  - **dirs** (uniquement côté Unix pour ajouter `$HOME`).

### 2.2 Composants principaux

- `TreeSizeApp` :
  - Implémente `eframe::App`.
  - Gère :
    - L’état UI (mode de vue, sélection, panneau latéral).
    - L’état du scan (en cours, annulé, terminé).
    - Le presse-papier interne pour copier/coller.
    - Les confirmations de suppression et les opérations différées (coller).
- `Node` :
  - Représente un dossier ou un fichier :
    - Agrégation des tailles, tri par taille, structure hiérarchique.
- Moteur de scan :
  - `scan_directory_parallel(root, cancel)` :
    - Construction de la hiérarchie.
    - Annulation coopérative via `cancel: &AtomicBool`.
  - `build_node(path, cancel)` :
    - Différencie fichier/dossier.
    - Scan récursif parallèle des sous-éléments.
- Rendu des vues :
  - Arborescence : `draw_node_recursive`.
  - Treemap : `draw_treemap` + `layout_treemap_rect`.

---

## 3. Installation & compilation

### 3.1 Prérequis

- **Rust stable** récent (via `rustup` recommandé).
- Plateformes visées :
  - Windows 10/11
  - Linux (Ubuntu, Debian, etc.)

### 3.2 Récupération du projet

```bash
git clone <url_du_repo> treesize_rust
cd treesize_rust
```

### 3.3 Compilation & exécution

Mode debug:
```bash
cargo run
```

Mode release(recommandé pour les gros scans):
```bash
cargo run --release
```

### 4. Utilisation
- Lancer l’application :
```bash
cargo run --release
```