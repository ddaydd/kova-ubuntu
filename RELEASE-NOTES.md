# Kova Ubuntu — Release Notes

## 2026-03-07

### Fix Super+V "v" parasite (round 2 — Ctrl+Shift+V/C)
- Le fix precedent (`event.text` au lieu de `logical_key`) ne suffit pas : sur X11/GNOME, `event.text` retourne `Some("v")` meme avec Super enfonce car Super ne modifie pas le keysym XKB
- Cause racine probable : GNOME grab la touche Super sur X11, donc `ModifiersChanged` n'arrive pas (ou pas a temps) et le keybinding `cmd+v` ne matche pas
- Fix : ajout de `Ctrl+Shift+V` (paste) et `Ctrl+Shift+C` (copy) comme keybindings additionnels dans `keybindings.rs` — standard Linux (gnome-terminal, kitty, alacritty)
- Les bindings `cmd+v`/`cmd+c` restent (fonctionnent si le systeme passe Super)
- Log de diagnostic ameliore pour tracer `super=true/false` et `text` sur les key events
- Fichiers modifies : `keybindings.rs`, `window.rs`

### Fix perte de session (4 bugs)
- Le fichier session.json etait supprime au chargement — un crash avant la premiere sauvegarde periodique (30s) perdait toute la session
- L'ecriture n'etait pas atomique — un crash pendant l'ecriture corrompait le fichier
- La sauvegarde a la sortie etait dans un thread — `process::exit()` pouvait tuer le thread avant la fin de l'ecriture
- Les fenetres etaient retirees de la map AVANT la sauvegarde — la session sauvee etait vide
- Fix : suppression du `remove_file` au load, ecriture atomique (tmp+rename), sauvegarde synchrone, save avant remove
- Fichiers modifies : `session.rs`, `app.rs`

### Debug selection texte decalee d'une ligne (en cours)
- La selection souris est decalee d'une ligne vers le bas
- Ajout de logs debug dans `mouse_to_grid` et `build_vertices` pour diagnostic
- En attente des valeurs de log pour identifier la cause
- Fichiers modifies : `window.rs`, `renderer/mod.rs`

## 2026-03-06

### Fix clic pour changer de terminal en vue grille
- Cliquer sur un terminal dans la grille (multi-tabs) ne changeait plus le focus
- Regression introduite par l'ajout de la selection texte a la souris : le clic demarrait la selection mais ne mettait plus a jour `active_tab` / `focused_pane`
- Fix : apres le hit-test du pane clique, on met a jour `active_project`, `active_tab` et `focused_pane`
- Fichier modifie : `window.rs` (handler MouseInput Left Pressed, zone pane)

### Fix Super+V colle un "v" parasite
- Super+V envoyait le caractere "v" au PTY en plus du contenu colle
- Cause : `handle_key_event` utilisait `event.logical_key` (qui donne `Character("v")` meme avec Super) pour ecrire au PTY
- Fix : utilise `event.text` (qui est `None` quand un modificateur comme Super est actif) pour l'envoi de caracteres reguliers
- Fichiers modifies : `input.rs` (nouveau param `text: Option<&str>`), `window.rs` (passe `event.text.as_deref()`)

### Fix selection souris decalee de 2 lignes
- La selection texte a la souris tombait ~2 lignes en dessous du clic
- Cause : `mouse_to_grid()` ne tenait pas compte du `y_offset` (decalage vertical quand le terminal n'est pas plein, via `y_offset_rows()`)
- Fix : soustrait `y_offset` dans le calcul de la ligne, meme formule que le renderer
- Fichier modifie : `window.rs` (mouse_to_grid)

### Fix coordonnees souris (scale factor)
- Les handlers souris melangeaient pixels logiques (souris / scale) et pixels physiques (cell_size, renderer)
- Avec un scale != 1 (fractional scaling Ubuntu), les clics sur tabs, la selection texte, le menu contextuel et le drag pointaient au mauvais endroit
- Fix : toutes les coordonnees souris restent en pixels physiques, coherent avec cell_size() et le renderer
- Fichier modifie : `window.rs` (CursorMoved, MouseInput left/right/release, pane_at, drag label)

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
