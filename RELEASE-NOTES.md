# Kova Ubuntu — Release Notes

## 2026-03-06

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
