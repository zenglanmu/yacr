// File API wiring for opening drawings.
export function createFileHost(wasmModule, { setStateKey, setStateText }) {
  function openDrawing(file) {
    return file.arrayBuffer().then((buffer) => {
      const bytes = new Uint8Array(buffer);
      try {
        setStateText(wasmModule.open_document_bytes(file.name, bytes));
      } catch (error) {
        console.error("yacr: open failed", error);
        setStateKey("host.open_failed", { error: String(error) });
      }
    });
  }

  function wireFilePickers() {
    const drawingInput = document.getElementById("file-input");
    document
      .getElementById("open-drawing")
      ?.addEventListener("click", () => drawingInput.click());
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
  }

  return { wireFilePickers };
}
