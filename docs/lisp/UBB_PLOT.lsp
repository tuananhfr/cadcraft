(defun ubb:available (symbol / value)
  (setq value (vl-catch-all-apply 'eval (list symbol)))
  (and (not (vl-catch-all-error-p value)) value))

(defun ubb:ubot-p () (if (ubb:available 'ubplot-scan) T nil))

(defun ubb:environment ()
  (cond ((ubb:ubot-p) "UbotCAD")
        ((progn (vl-load-com) (ubb:available 'vlax-ename->vla-object)) "AutoCAD")
        (T "Unknown")))

(defun ubb:fail (message)
  (prompt (strcat "\nUBB_PLOT: " message))
  (exit))

(defun ubb:quote (text / result i ch code hex)
  (if (> (strlen text) 1048576) (ubb:fail "Text exceeds the reference routine limit"))
  (setq result "\"" i 1 hex "0123456789abcdef")
  (repeat (strlen text)
    (setq ch (substr text i 1) code (ascii ch) i (1+ i))
    (setq result (strcat result
      (cond ((= ch "\"") "\\\"") ((= ch "\\") "\\\\")
            ((< code 32) (strcat "\\u00" (substr hex (1+ (fix (/ code 16))) 1)
                                             (substr hex (1+ (rem code 16)) 1)))
            (T ch)))))
  (strcat result "\""))

(defun ubb:number (value / text)
  (setq text (rtos value 2 8))
  (cond ((= (substr text 1 1) ".") (setq text (strcat "0" text)))
        ((= (substr text 1 2) "-.") (setq text (strcat "-0" (substr text 2)))))
  (if (= (substr text (strlen text) 1) ".") (setq text (strcat text "0")))
  text)

(defun ubb:join (items / text separator item)
  (setq text "" separator "")
  (foreach item items
    (setq text (strcat text separator item) separator ","))
  text)

(defun ubb:point-json (point)
  (strcat "[" (ubb:number (car point)) "," (ubb:number (cadr point)) "]"))

(defun ubb:effective-name (entity / object)
  (if (ubb:ubot-p) (getpropertyvalue entity "EffectiveName")
    (progn (setq object (vlax-ename->vla-object entity)) (vla-get-EffectiveName object))))

(defun ubb:rotation (entity)
  (if (ubb:ubot-p) (getpropertyvalue entity "Rotation")
    (vla-get-Rotation (vlax-ename->vla-object entity))))

(defun ubb:attribs (entity / next data result count)
  (if (ubb:ubot-p) (ubplot-attribs entity)
    (progn
      (setq result nil count 0)
      (if (= 1 (cdr (assoc 66 (entget entity))))
        (progn
          (setq next (entnext entity))
          (while (and next (< count 10000))
            (setq data (entget next) count (1+ count))
            (if (= (cdr (assoc 0 data)) "SEQEND") (setq next nil)
              (progn
                (if (= (cdr (assoc 0 data)) "ATTRIB")
                  (setq result (cons (cons (cdr (assoc 2 data)) (cdr (assoc 1 data))) result)))
                (setq next (entnext next)))))
          (if next (ubb:fail "Attribute count exceeds 10000"))))
      (reverse result))))

(defun ubb:native-templates (/ result entry name)
  (foreach entry (ubplot-templates)
    (foreach name (cdr (assoc "names" (cdr entry)))
      (setq result (cons
        (list name (cons 'id (car entry))
          (cons 'frame (cdr (assoc "frame" (cdr entry))))
          (cons 'paper (cdr (assoc "paper" (cdr entry))))
          (list 'sheet_id_tags (cdr (assoc "sheet_id_tags" (cdr entry))))) result))))
  (reverse result))

(defun ubb:refresh-templates ()
  (if (ubb:ubot-p) (setq *ubb-templates* (ubb:native-templates)))
  (if (not *ubb-templates*) (ubb:fail "Load UBB_PLOT_TEMPLATES.lsp first")))

(defun ubb:template-id (entry / id split)
  (setq id (cdr (assoc 'id (cdr entry))) split (vl-string-search "@" id))
  (if split (substr id 1 split) id))

(defun ubb:template-version (entry / id split)
  (setq id (cdr (assoc 'id (cdr entry))) split (vl-string-search "@" id))
  (if split (atoi (substr id (+ split 2))) 0))

(defun ubb:template (name / result entry)
  (foreach entry *ubb-templates*
    (if (and (wcmatch (strcase name) (strcase (car entry)))
             (or (not result)
                 (and (= (ubb:template-id result) (ubb:template-id entry))
                      (> (ubb:template-version entry) (ubb:template-version result)))))
      (setq result entry)))
  result)

(defun ubb:selection (/ choice selected kept i entity)
  (initget "All Select")
  (setq choice (getkword "\nFrames [All/Select] <Select>: "))
  (setq selected (if (= choice "All") (ssget "X" '((0 . "INSERT"))) (ssget '((0 . "INSERT")))))
  (setq kept (ssadd) i 0)
  (if selected
    (progn
      (if (> (sslength selected) 10000) (ubb:fail "Selection exceeds 10000 INSERTs"))
      (repeat (sslength selected)
        (setq entity (ssname selected i) i (1+ i))
        (if (ubb:template (ubb:effective-name entity)) (ssadd entity kept)))))
  kept)

(defun ubb:world (point base position sx sy angle / x y)
  (setq x (* sx (- (car point) (car base))) y (* sy (- (cadr point) (cadr base))))
  (list (+ (car position) (- (* x (cos angle)) (* y (sin angle))))
        (+ (cadr position) (+ (* x (sin angle)) (* y (cos angle))))))

(defun ubb:outline (frame base position sx sy angle / result point direction score best start i)
  (foreach point frame (setq result (cons (ubb:world point base position sx sy angle) result)))
  (setq result (reverse result))
  (if (< (* sx sy) 0.0) (setq result (list (car result) (cadddr result) (caddr result) (cadr result))))
  ; Match the native counter-clockwise origin using the transformed local up direction.
  (setq direction (if (< sy 0.0) -1.0 1.0) i 0 start 0)
  (foreach point result
    (setq score (* direction (+ (* (car point) (- (cos angle) (sin angle)))
                               (* (cadr point) (+ (sin angle) (cos angle))))))
    (if (or (not best) (< score best)) (setq best score start i))
    (setq i (1+ i)))
  (repeat start (setq result (append (cdr result) (list (car result)))))
  result)

(defun ubb:sheet-json (entity template order / data frame tags attrs tag id name base position sx sy angle outline width height issue)
  (setq data (entget entity) frame (cdr (assoc 'frame (cdr template)))
        tags (cadr (assoc 'sheet_id_tags (cdr template)))
        attrs (ubb:attribs entity) name (ubb:effective-name entity))
  (foreach tag tags
    (if (and (not id) (cdr (assoc tag attrs)) (/= (cdr (assoc tag attrs)) ""))
      (setq id (cdr (assoc tag attrs)))))
  (if (not id)
    (setq id (strcat "S-" (cond ((< order 10) "00") ((< order 100) "0") (T "")) (itoa order))))
  (setq position (cdr (assoc 10 data)) angle (ubb:rotation entity)
        sx (cond ((cdr (assoc 41 data))) (T 1.0)) sy (cond ((cdr (assoc 42 data))) (T 1.0))
        base (cdr (assoc 10 (tblsearch "BLOCK" (cdr (assoc 2 data))))))
  (if (not base) (setq base '(0.0 0.0)))
  (setq issue (or (/= (length frame) 4) (= sx 0.0) (= sy 0.0) (> (abs (- (abs sx) (abs sy))) 0.000001)))
  (if issue
    (setq outline '((0.0 0.0) (0.0 0.0) (0.0 0.0) (0.0 0.0)) width 297.0 height 420.0)
    (progn
      (setq outline (ubb:outline frame base position sx sy angle)
            width (distance (car frame) (cadr frame)) height (distance (cadr frame) (caddr frame)))))
  (strcat "{\"sheet_id\":" (ubb:quote id) ",\"order\":" (itoa order) ",\"mode\":\"block\",\"source\":{\"kind\":\"block\","
    "\"block_handle\":" (ubb:quote (cdr (assoc 5 data))) ",\"effective_name\":" (ubb:quote name)
    ",\"outline\":[" (ubb:join (mapcar 'ubb:point-json outline)) "],\"rotation_deg\":" (ubb:number (* angle (/ 180.0 pi))) "},"
    "\"frame_template_id\":" (ubb:quote (cdr (assoc 'id (cdr template))))
    ",\"paper_mm\":[" (ubb:number (min width height)) "," (ubb:number (max width height)) "]"
    ",\"landscape\":" (if (> width height) "true" "false") ",\"plot_scale\":"
    (if issue "\"fit\"" (strcat "\"1:" (ubb:number (abs sx)) "\""))
    ",\"status\":" (if issue "\"BLOCKED\"" "\"NEEDS_PREVIEW\"") ",\"issues\":"
    (if issue "[{\"severity\":\"BLOCK\",\"code\":\"FRAME_AMBIGUOUS\",\"message\":\"Use the native adapter or select a Window\"}]" "[]") "}"))

(defun ubb:plan-json (selection / sheets i entity template)
  (ubb:refresh-templates)
  (if (and (ubb:ubot-p) (not *ubb-portable*)) (ubplot-scan selection)
    (progn
      (setq i 0 sheets nil)
      (if selection
        (progn
          (if (> (sslength selection) 10000) (ubb:fail "Selection exceeds 10000 INSERTs"))
          (repeat (sslength selection)
            (setq entity (ssname selection i) i (1+ i)
                  template (ubb:template (ubb:effective-name entity)))
            (if template (setq sheets (cons (ubb:sheet-json entity template (1+ (length sheets))) sheets))))))
      (strcat "{\"schema_version\":\"1.0\",\"job_id\":\"lisp\",\"source_hash\":\"\",\"sheets\":["
              (ubb:join (reverse sheets)) "],\"issues\":[]}"))))

(defun ubb:output-path (/ directory)
  (setq directory (if (ubb:ubot-p) *ubb-output-dir* (getvar "DWGPREFIX")))
  (if (or (not directory) (= directory ""))
    (setq directory (getstring T "\nAllowed output folder: ")))
  (if (or (not directory) (= directory "")) (ubb:fail "An output folder is required"))
  (strcat directory "/UBB_PLOT.json"))

(defun C:UBB_PLOT (/ selection plan file path)
  (ubb:refresh-templates)
  (setq selection (ubb:selection))
  (if (> (sslength selection) 0)
    (progn
      (setq plan (ubb:plan-json selection) path (ubb:output-path) file (open path "w"))
      (if (not file) (ubb:fail "Could not open UBB_PLOT.json in the allowed folder"))
      (write-line plan file)
      (close file)
      (if (and (ubb:ubot-p) (not *ubb-portable*))
        (setq *ubb-manifest* (ubplot-publish plan))
        (if (= (ubb:environment) "AutoCAD") (prompt "\nPlot on AutoCAD: not verified yet"))))
    (prompt "\nNo matching INSERTs selected"))
  (princ))

(defun C:UBB_PLOT_CHECK (/ environment required symbol missing version)
  (setq environment (ubb:environment) version (getvar "ACADVER"))
  (prompt (strcat "\nEnvironment: " environment "\nVersion: " (ver) "\nACADVER: " (if version version "unknown")))
  (setq required '(ssget ssadd sslength ssname entget tblsearch wcmatch rtos open write-line close
                   initget getkword getstring strcase strlen substr ascii chr itoa atoi
                   distance sin cos mapcar reverse assoc vl-string-search vl-symbol-name
                   vl-catch-all-apply vl-catch-all-error-p eval ver))
  (if (= environment "UbotCAD")
    (setq required (append required '(getpropertyvalue ubplot-attribs ubplot-scan ubplot-publish ubplot-templates)))
    (setq required (append required '(entnext vl-load-com vlax-ename->vla-object vla-get-EffectiveName vla-get-Rotation))))
  (foreach symbol required
    (if (not (ubb:available symbol)) (setq missing (cons (vl-symbol-name symbol) missing))))
  (if missing (prompt (strcat "\nMissing functions: " (ubb:join (reverse missing))))
    (prompt "\nDependencies: OK"))
  (princ))

(if (not (boundp '*ubb-portable*)) (setq *ubb-portable* nil))
(if (not (boundp '*ubb-output-dir*)) (setq *ubb-output-dir* nil))
(if (ubb:ubot-p) (setq *ubb-templates* (ubb:native-templates)))
(princ)
