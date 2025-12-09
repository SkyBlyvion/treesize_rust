use std::path::{Path, PathBuf};
use std::thread;

use crossbeam_channel::{unbounded, Receiver};
use eframe::{egui, NativeOptions};
use rayon::prelude::*;

fn main() -> eframe::Result<()> {
    let native_options = NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(egui::vec2(1000.0, 700.0)),
        ..Default::default()
    };

    eframe::run_native(
        "TreeSize Rust (Windows / Linux)",
        native_options,
        Box::new(|_cc| Box::new(TreeSizeApp::default())),
    )
}

#[derive(Debug, Clone)]
struct Node {
    name: String,
    path: PathBuf,
    is_dir: bool,
    size: u64,
    file_count: u64,
    children: Vec<Node>,
}

impl Node {
    fn new_dir(name: String, path: PathBuf, children: Vec<Node>) -> Self {
        let mut size = 0;
        let mut file_count = 0;

        for child in &children {
            size += child.size;
            file_count += child.file_count;
        }

        let mut children = children;
        children.sort_by(|a, b| b.size.cmp(&a.size));

        Self {
            name,
            path,
            is_dir: true,
            size,
            file_count,
            children,
        }
    }

    fn new_file(name: String, path: PathBuf, size: u64) -> Self {
        Self {
            name,
            path,
            is_dir: false,
            size,
            file_count: 1,
            children: Vec::new(),
        }
    }
}

#[derive(Debug)]
struct ScanResult {
    root_path: PathBuf,
    root_node: Option<Node>,
    error: Option<String>,
}

#[derive(Default)]
struct TreeSizeApp {
    root_path: Option<PathBuf>,
    root_node: Option<Node>,
    is_scanning: bool,
    status: String,
    scan_receiver: Option<Receiver<ScanResult>>,
}

impl eframe::App for TreeSizeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Récupère le résultat du scan si disponible
        if self.is_scanning {
            if let Some(rx) = &self.scan_receiver {
                if let Ok(result) = rx.try_recv() {
                    self.is_scanning = false;
                    self.scan_receiver = None;

                    if let Some(err) = result.error {
                        self.root_node = None;
                        self.status = format!("Erreur pendant le scan : {err}");
                    } else {
                        self.root_node = result.root_node;
                        self.status = format!(
                            "Scan terminé pour : {}",
                            result.root_path.to_string_lossy()
                        );
                    }

                    ctx.request_repaint();
                }
            }
        }

        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.button("Choisir un dossier…").clicked() && !self.is_scanning {
                    if let Some(path) = pick_directory() {
                        self.root_path = Some(path.clone());
                        self.status = format!(
                            "Dossier sélectionné : {}",
                            path.to_string_lossy()
                        );
                    }
                }

                if ui.button("Scanner").clicked() && !self.is_scanning {
                    if let Some(path) = self.root_path.clone() {
                        self.start_scan(path);
                    } else {
                        self.status = "Aucun dossier sélectionné".to_string();
                    }
                }

                if ui.button("Arrêter (non implémenté)").clicked() {
                    // À implémenter si tu veux un cancel de scan
                }

                ui.separator();

                if let Some(path) = &self.root_path {
                    ui.label(format!("Racine : {}", path.to_string_lossy()));
                } else {
                    ui.label("Racine : (aucun dossier)");
                }
            });
        });

        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            if self.is_scanning {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(&self.status);
                });
            } else {
                ui.label(&self.status);
            }
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(root) = &self.root_node {
                let total_size = root.size;
                let total_files = root.file_count;

                ui.heading("Résultats du scan");
                ui.horizontal(|ui| {
                    ui.label(format!(
                        "Taille totale : {}",
                        format_bytes(total_size)
                    ));
                    ui.separator();
                    ui.label(format!("Nombre de fichiers : {}", total_files));
                });

                ui.separator();
                ui.heading("Arborescence");

                ui.add_space(8.0);

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        draw_node_recursive(ui, root, total_size, 0);
                    });
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label("Sélectionne un dossier puis lance un scan.");
                });
            }
        });
    }
}

impl TreeSizeApp {
    fn start_scan(&mut self, path: PathBuf) {
        self.is_scanning = true;
        self.status = format!(
            "Scan en cours pour : {}",
            path.to_string_lossy()
        );
        self.root_node = None;

        let (tx, rx) = unbounded::<ScanResult>();
        self.scan_receiver = Some(rx);

        thread::spawn(move || {
            let result = scan_directory_parallel(&path);
            let _ = tx.send(result);
        });
    }
}

/// Dessin récursif d'un Node en arbre.
fn draw_node_recursive(
    ui: &mut egui::Ui,
    node: &Node,
    root_size: u64,
    indent_level: usize,
) {
    let indent = 16.0 * indent_level as f32;
    let percentage = if root_size > 0 {
        (node.size as f64 / root_size as f64) * 100.0
    } else {
        0.0
    };

    let label = if node.is_dir {
        format!(
            "{} ({} | {} fichiers | {:.2}%)",
            node.name,
            format_bytes(node.size),
            node.file_count,
            percentage
        )
    } else {
        format!(
            "{} ({}, {:.2}%)",
            node.name,
            format_bytes(node.size),
            percentage
        )
    };

    if node.is_dir {
        // API egui récente : on retire .selectable(false)
        let header = egui::CollapsingHeader::new(label)
            .default_open(indent_level == 0)
            .id_source(node.path.to_string_lossy().to_string());

        header.show(ui, |ui| {
            for child in &node.children {
                ui.horizontal(|ui| {
                    ui.add_space(indent + 10.0);
                    draw_node_recursive(ui, child, root_size, indent_level + 1);
                });
            }
        });
    } else {
        ui.horizontal(|ui| {
            ui.add_space(indent + 10.0);
            ui.label(label);
        });
    }
}

/// Utilise une boîte de dialogue native pour choisir un dossier.
fn pick_directory() -> Option<PathBuf> {
    rfd::FileDialog::new().pick_folder()
}

/// Scan récursif avec parallélisation :
fn scan_directory_parallel(root: &Path) -> ScanResult {
    if !root.exists() {
        return ScanResult {
            root_path: root.to_path_buf(),
            root_node: None,
            error: Some("Le dossier n'existe pas".to_string()),
        };
    }

    if !root.is_dir() {
        return ScanResult {
            root_path: root.to_path_buf(),
            root_node: None,
            error: Some("Chemin sélectionné n'est pas un dossier".to_string()),
        };
    }

    // Première passe : on récupère les entrées de premier niveau
    let mut direct_children: Vec<PathBuf> = Vec::new();
    match root.read_dir() {
        Ok(read_dir) => {
            for entry in read_dir.flatten() {
                direct_children.push(entry.path());
            }
        }
        Err(e) => {
            return ScanResult {
                root_path: root.to_path_buf(),
                root_node: None,
                error: Some(format!("Impossible de lire le dossier racine : {e}")),
            };
        }
    }

    // Parallélise le traitement de chaque enfant direct (dossier ou fichier)
    let children_nodes: Vec<Node> = direct_children
        .par_iter()
        .filter_map(|path| build_node(path).ok())
        .collect();

    let name = root
        .file_name()
        .map(|os| os.to_string_lossy().to_string())
        .unwrap_or_else(|| root.to_string_lossy().to_string());

    let root_node = Node::new_dir(name, root.to_path_buf(), children_nodes);

    ScanResult {
        root_path: root.to_path_buf(),
        root_node: Some(root_node),
        error: None,
    }
}

/// Construit un Node (fichier ou dossier) pour un chemin donné.
/// Pour les dossiers, on scanne récursivement (en parallèle pour les sous-éléments).
fn build_node(path: &Path) -> Result<Node, String> {
    if path.is_file() {
        let size = path.metadata().map(|m| m.len()).unwrap_or(0);
        let name = path
            .file_name()
            .map(|os| os.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string());
        Ok(Node::new_file(name, path.to_path_buf(), size))
    } else if path.is_dir() {
        let name = path
            .file_name()
            .map(|os| os.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string());

        let mut entries: Vec<PathBuf> = Vec::new();
        if let Ok(read_dir) = path.read_dir() {
            for entry in read_dir.flatten() {
                entries.push(entry.path());
            }
        }

        let children: Vec<Node> = entries
            .par_iter()
            .filter_map(|p| build_node(p).ok())
            .collect();

        Ok(Node::new_dir(name, path.to_path_buf(), children))
    } else {
        Err("Type de fichier non pris en charge".to_string())
    }
}

/// Format taille en bytes en Ko/Mo/Go lisible.
fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["o", "Ko", "Mo", "Go", "To"];

    let mut size = bytes as f64;
    let mut unit = 0;

    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }

    format!("{:.1} {}", size, UNITS[unit])
}
