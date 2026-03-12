# Kova Linux — Release Notes

## v1.6.0 — 2026-03-12

### Notifications améliorées
- Toast in-app pour BEL (visible même en fullscreen)
- `--urgency=critical` sur les `notify-send` (BEL + command completion)
- Son de notification via `canberra-gtk-play` : son `bell` pour BEL, son `complete` pour command completion

### Extension GNOME fullscreen-notifications
- Extension GNOME locale (`~/.local/share/gnome-shell/extensions/fullscreen-notifications@kova/`)
- Patche `_updateState` de messageTray pour afficher les notifications même en fullscreen
- Compatible GNOME Shell 46

### Fix grille : 4 panes en 2x2 au lieu de 3+1
- L'algorithme Termix utilisait `.round()` sur le nombre de colonnes, ce qui arrondissait vers le haut (ex: `sqrt(4*1.78) = 2.67 → 3 cols`)
- Remplacé par un floor implicite (`as usize`) : 4 panes donne maintenant correctement 2x2

### Project sidebar (barre latérale gauche)
- La barre de projets passe du haut de la fenêtre à une sidebar verticale à gauche
- Largeur fixe de 12 cellules, hauteur pleine fenêtre (hors status bar)
- Filtres (All, Claude, Terminal) et projets listés verticalement
- Accent vert sur la droite pour l'élément actif (au lieu du bas)
- Bouton "+" en bas de la liste
- Tab bar positionnée à droite de la sidebar (y=0)
- Pane area démarre à `x = sidebar_w` — plus d'espace vertical gagné
- Hit-test souris adapté : clic gauche, droit, drag & drop fonctionnent sur l'axe Y

### Sidebar élargie + version label déplacé
- Sidebar élargie de 12 à 36 cellules (noms de projets plus lisibles)
- Menu contextuel aligné sur la nouvelle largeur
- Label version déplacé de la tab bar vers la global bar (à gauche)

### Optimisation rendu GPU
- Réutilisation du Vec de vertices entre frames (évite une allocation par frame)
- Préférence GPU `HighPerformance` (utilise le GPU dédié si disponible)

## v1.5.0 — 2026-03-11

### Détection Claude Code et filtres project bar
- Détection automatique des panes exécutant Claude Code via `/proc/<pid>/cmdline` (basename = "claude")
- Nouveaux onglets filtres **Claude** et **Terminal** dans la project bar (apparaissent quand un pane Claude est actif + 2 projets min)
- **Claude** : affiche en grille uniquement les tabs contenant un pane Claude
- **Terminal** : affiche uniquement les tabs sans pane Claude
- Refactoring de `show_all: bool` → `ViewMode { Project, All, Claude, Terminal }` enum
- Auto-fallback vers le mode Project si le filtre ne matche plus rien
- `Pty::is_claude()`, `Pane::is_claude()`, `SplitTree::any_pane()` ajoutés
- `visible_tabs()` et `is_grid_view()` centralisent la logique de vue

## v1.4.0 — 2026-03-09

### Notification de fin de commande (OSC 133)
- Détecte début/fin de commande via OSC 133;C/D (FinalTerm shell integration)
- Si la commande a duré > 5s → notification système (`notify-send`) + toast in-app
- Seuil configurable : `terminal.notify_threshold_secs` (0 = désactivé)
- Notifications envoyées même quand la fenêtre est focused
- Shell integration bash ajoutée dans `~/.bashrc` (OSC 133 + OSC 7 + OSC 7777)

### v1.3.0 — Améliorations CLI et logs
- Flag `--version` / `-V` ajouté
- Version affichée dans `--help` et overlay F1
- Fichier log en mode append (ne s'écrase plus au redémarrage)
- Version loggée au démarrage
- Bell notification envoyée même fenêtre focused (suppression du check `!window_focused`)

### Toast in-app (F2 Save Session)
- Affiche "Session saved" en bas-centre de l'écran pendant 2s avec fade out
- Système générique `show_toast(msg)` réutilisable pour d'autres actions

### Notifications système (BEL)
- Quand un pane reçoit un BEL (`\a`) et que la fenêtre Kova n'a pas le focus → notification desktop via `notify-send`
- Cooldown de 5s pour éviter le spam
- Utile pour être notifié quand Claude Code termine une tâche

## v1.2.0 — 2026-03-07

### Onglet "All" permanent dans la project bar
- L'onglet "All" apparaît automatiquement dans la project bar dès qu'il y a 2+ projets
- Cliquer dessus toggle la vue grille de tous les terminaux
- Se désactive automatiquement quand il ne reste qu'un seul projet

### Fix Super+V "v" parasite (round 2 — Ctrl+Shift+V/C)
- Le fix precedent (`event.text` au lieu de `logical_key`) ne suffit pas : sur X11/GNOME, `event.text` retourne `Some("v")` meme avec Super enfonce car Super ne modifie pas le keysym XKB
- Cause racine probable : GNOME grab la touche Super sur X11, donc `ModifiersChanged` n'arrive pas (ou pas a temps) et le keybinding `cmd+v` ne matche pas
- Fix : ajout de `Ctrl+Shift+V` (paste) et `Ctrl+Shift+C` (copy) comme keybindings additionnels dans `keybindings.rs` — standard Linux (gnome-terminal, kitty, alacritty)
- Les bindings `cmd+v`/`cmd+c` restent (fonctionnent si le systeme passe Super)

### Fix perte de session (4 bugs)
- Le fichier session.json etait supprime au chargement — un crash avant la premiere sauvegarde periodique (30s) perdait toute la session
- L'ecriture n'etait pas atomique — un crash pendant l'ecriture corrompait le fichier
- La sauvegarde a la sortie etait dans un thread — `process::exit()` pouvait tuer le thread avant la fin de l'ecriture
- Les fenetres etaient retirees de la map AVANT la sauvegarde — la session sauvee etait vide
- Fix : suppression du `remove_file` au load, ecriture atomique (tmp+rename), sauvegarde synchrone, save avant remove

### Aide F1 reorganisee par sections
- Overlay d'aide organise en 5 sections : Tabs, Splits, Projects & Windows, Editing, System
- Raccourcis manquants ajoutes : Super+0 (show all), Super+E/Shift+E (root splits), Super+Shift+R (rename), Super+Shift+T (detach), Super+Shift+M (merge), Ctrl+Shift+C/V

### Fix pane actif absent en mode "Show All"
- En mode Show All (Super+0), aucun pane n'etait marque comme actif/focused (pas de bordure)
- Cause : tous les tabs etaient marques `is_active_tab = false` en mode show_all
- Fix : on identifie le tab actif du projet actif pour marquer le bon pane comme focused

### Debug selection texte decalee d'une ligne (en cours)
- Ajout de logs debug dans `mouse_to_grid` et `build_vertices` pour diagnostic

## 2026-03-06

### Vue "All terminals" (Super+0)
- Nouveau mode grille affichant tous les terminaux de tous les projets
- Entree "All" dans la project bar (premier slot)
- Clic sur un pane = switch au projet/tab correspondant et quitte le mode All
- Tab bar masquee en mode All

### Rename project (clic droit)
- Clic droit sur un projet dans la project bar = renommer
- `custom_name` sauvegarde/restaure avec la session

### Selection de texte a la souris
- Clic gauche dans un pane = debut de selection, drag = extension, relachement = fin

### Menu contextuel (clic droit dans un pane)
- Copy / Paste via menu contextuel, Copy grise si pas de selection

### Fix clic pour changer de terminal en vue grille
- Regression : le clic demarrait la selection mais ne mettait plus a jour le focus
- Fix : apres le hit-test, on met a jour `active_project`, `active_tab` et `focused_pane`

### Fix Super+V colle un "v" parasite
- Cause : `handle_key_event` utilisait `event.logical_key` au lieu de `event.text`
- Fix : utilise `event.text` (None quand un modificateur comme Super est actif)

### Fix selection souris decalee de 2 lignes
- `mouse_to_grid()` ne tenait pas compte du `y_offset`

### Fix coordonnees souris (scale factor)
- Les handlers souris melangeaient pixels logiques et physiques
- Fix : tout en pixels physiques, coherent avec cell_size() et le renderer

## 2026-03-05

### Refonte gestion des projets et tabs
- Les barres projet et tab sont toujours visibles (meme avec 1 seul element)
- Bouton "+" sur la barre projets pour creer un nouveau projet
- Bouton "+" sur la barre tabs pour ajouter un terminal au projet courant
- Suppression du regroupement automatique par repertoire : chaque `open_project` cree un tab orphelin dans le projet actif
- Le clic droit "Open with Kova" restaure la session ET ajoute le repertoire comme nouveau tab

### Vue grille automatique
- Quand un projet a plusieurs tabs, ils sont tous affiches en grille (algorithme Termix : `cols = round(sqrt(n * W/H))`)
- Cliquer sur un pane dans la grille change le focus vers ce tab/pane
- Les panes sont redimensionnes selon leur cellule de grille

### Drag & drop de tabs
- Glisser un tab depuis la barre de tabs et le deposer sur un projet dans la barre de projets
- Label flottant avec le nom du tab suit la souris pendant le drag
- Seuil de 5px pour distinguer clic et drag

### Deplacer un tab entre projets (clavier)
- `Super+Alt+Shift+Left/Right` deplace le tab actif vers le projet precedent/suivant

### Corrections
- Scroll molette inverse corrige (suppression de la negation `-lines` hérité du port macOS)
- Save session passe sur F2 (F-key safe, pas de conflit terminal)
- F2 ajoute au KeyType enum + mappings
- Aide F1 mise a jour avec F2/Save session et Move tab to project

## 2026-03-04

### F11 fullscreen + F1 help
- F11 toggle fullscreen
- F1 toggle help overlay

### Desktop integration (`--install`)
- `kova --install` : symlink `~/.local/bin/kova`, fichier `.desktop`, action Nemo
- `kova --install --autostart` : idem + lancement au login
- `kova --uninstall` : supprime tout
- `kova --help` : affiche toutes les options CLI

### Argument repertoire
- `kova /chemin` ouvre le terminal dans le dossier specifie
- Fonctionne avec "Ouvrir avec" depuis le gestionnaire de fichiers

### Fix flash au demarrage
- Fenetre creee invisible, premier frame rendu, puis affichee

### Projects (groupement de tabs par dossier)
- Structure `Project` : regroupe les tabs par dossier racine
- Project bar au-dessus de la tab bar
- Session sauvegarde/restaure les projets (backward compat v2)

### Instance unique (IPC socket Unix)
- Socket `/run/user/$UID/kova.sock`
- `kova /chemin` envoie le path a l'instance existante

## v1.0.0 — Port Linux (2026-03-04)

Fork du terminal macOS [Kova](https://github.com/ddaydd/kova) porte vers Linux.

### Stack

| macOS | Linux |
|---|---|
| Metal | wgpu (Vulkan/OpenGL) |
| AppKit (NSWindow, NSView) | winit (X11/Wayland) |
| CoreText | FreeType + fontconfig |
| NSPasteboard | arboard (X11/Wayland clipboard) |

### Rendu texte
- LCD subpixel rendering via FreeType `TARGET_LCD`
- Emoji couleur supportes via `FT_LOAD_COLOR` (BGRA)
- Box-drawing characters rendus par code

### Raccourcis clavier
- **Super** (touche Win) remplace **Cmd**
- **Alt** remplace **Option**
- Tous les raccourcis sont configurables via `~/.config/kova/config.toml`
