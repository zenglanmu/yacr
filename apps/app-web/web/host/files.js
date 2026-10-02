// File API wiring and annotation download confirmation (B05/B07).
export function createFileHost(wasmModule, { t, setStateKey, setStateText }) {
  function promptUnsavedDecision() {
    const choice = window.prompt(t("host.open_needs_decision"), "cancel");
    if (choice === null) return null;
    const normalized = choice.trim().toLowerCase();
    return ["save", "recovery", "preserve", "discard", "cancel"].includes(
      normalized,
    )
      ? normalized
      : null;
  }

  function openDrawing(file) {
    return file.arrayBuffer().then((buffer) => {
      const bytes = new Uint8Array(buffer);
      let decision = "discard";
      if (wasmModule.open_requires_decision()) {
        decision = promptUnsavedDecision();
        if (!decision) {
          setStateKey("host.cancelled_open");
          return;
        }
      }
      try {
        setStateText(
          wasmModule.open_document_bytes_decided(file.name, bytes, decision),
        );
      } catch (error) {
        console.error("yacr: open failed", error);
        setStateKey("host.open_failed", { error: String(error) });
      }
    });
  }

  function exportAnnotations() {
    let bundle;
    try {
      bundle = wasmModule.annotation_export_json();
    } catch (error) {
      setStateKey("host.export_failed", { error: String(error) });
      return;
    }
    const blob = new Blob([bundle.json], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.download = "annotations.cadnotes.json";
    anchor.click();
    URL.revokeObjectURL(url);
    try {
      wasmModule.annotation_confirm_export(bundle.revision);
      setStateKey("host.exported_bytes", { bytes: bundle.json.length });
    } catch (error) {
      setStateKey("host.export_confirm_failed", { error: String(error) });
    }
  }

  function wireFilePickers() {
    const drawingInput = document.getElementById("file-input");
    drawingInput.addEventListener("change", async () => {
      const file = drawingInput.files && drawingInput.files[0];
      if (!file) return;
      try {
        await openDrawing(file);
      } catch (error) {
        console.error("yacr: open failed", error);
        setStateKey("host.open_failed", { error: String(error) });
      } finally {
        drawingInput.value = "";
      }
    });

    const annotationInput = document.getElementById("annotation-input");
    annotationInput.addEventListener("change", async () => {
      const file = annotationInput.files && annotationInput.files[0];
      if (!file) return;
      try {
        const count = wasmModule.annotation_import_json(await file.text());
        setStateKey("host.imported_count", { count });
      } catch (error) {
        console.error("yacr: annotation import failed", error);
        setStateKey("host.import_failed", { error: String(error) });
      } finally {
        annotationInput.value = "";
      }
    });
  }

  return { exportAnnotations, wireFilePickers };
}
