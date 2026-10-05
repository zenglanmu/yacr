//! Native CPU preparation over immutable inputs, with a bounded latest mailbox.
use super::*;
use cad_app::background::LatestTask;
use cad_app::render_scene::{CadSceneController, OverlayInputs};

#[derive(Clone, PartialEq, Eq)]
struct InputKey {
    drawing: usize,
    document: DocumentId,
    space: SpaceSelection,
    fonts: usize,
    layers: u64,
    overlays: u64,
}

pub(super) struct PreparationInput {
    pub drawing: Option<Arc<DrawingDatabase>>,
    pub document: DocumentId,
    pub space: SpaceSelection,
    pub fonts: Option<Arc<FontEngine>>,
    pub layers: LayerOverrideSet,
    pub overlays: OverlayInputs,
}

fn pointer<T>(value: &Option<Arc<T>>) -> usize {
    value
        .as_ref()
        .map_or(0, |value| Arc::as_ptr(value) as usize)
}

impl PreparationInput {
    fn key(&self) -> InputKey {
        InputKey {
            drawing: pointer(&self.drawing),
            document: self.document,
            space: self.space,
            fonts: pointer(&self.fonts),
            layers: self.layers.fingerprint(),
            overlays: controller::overlay_fingerprint(&self.overlays),
        }
    }
}

type Completion = CadResult<CadSceneController>;

#[derive(Default)]
pub(super) struct NativePreparation {
    worker: Option<LatestTask<PreparationInput, Completion>>,
    requested: Option<InputKey>,
    published: Option<InputKey>,
    busy: bool,
}

impl NativePreparation {
    pub fn update(&mut self, input: PreparationInput) -> CadResult<Option<CadSceneController>> {
        if self.worker.is_none() {
            let mut controller = CadSceneController::default();
            let mut fonts = None;
            self.worker = Some(
                LatestTask::new("cad-scene-preparation", move |input: PreparationInput| {
                    if pointer(&fonts) != pointer(&input.fonts) {
                        controller.fonts_changed();
                    }
                    fonts = input.fonts.clone();
                    controller.prepare_shared_with_overlays(
                        input.drawing,
                        input.document,
                        input.fonts,
                        &input.layers,
                        input.space,
                        &input.overlays,
                    )?;
                    Ok(controller.clone())
                })
                .map_err(|e| CadError::InvalidInput(e.to_string()))?,
            );
        }
        let key = input.key();
        let worker = self.worker.as_ref().unwrap();
        if self.requested.as_ref() != Some(&key) {
            self.requested = Some(key);
            self.busy = true;
            worker.submit(input);
        }
        if let Some((_, result)) = worker.take_completed() {
            self.busy = false;
            let controller = result?;
            self.published = self.requested.clone();
            return Ok(Some(controller));
        }
        if let Some((_, error)) = worker.take_failure() {
            self.busy = false;
            return Err(CadError::Invariant(error));
        }
        Ok(None)
    }

    pub fn busy(&self) -> bool {
        self.busy
    }
    pub fn stop(&mut self) {
        self.worker = None;
        self.busy = false;
        self.requested = None;
        self.published = None;
    }
    pub fn current(&self, drawing: &Option<Arc<DrawingDatabase>>) -> bool {
        !self.busy
            && self.published == self.requested
            && self
                .published
                .as_ref()
                .is_some_and(|key| key.drawing == pointer(drawing))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn input(drawing: Arc<DrawingDatabase>) -> PreparationInput {
        PreparationInput {
            drawing: Some(drawing),
            document: DocumentId(1),
            space: SpaceSelection::Model,
            fonts: None,
            layers: LayerOverrideSet::new(),
            overlays: OverlayInputs::default(),
        }
    }

    #[test]
    fn latest_same_id_database_replacement_and_overlay_update_preserve_cache_identity() {
        let first = cad_app::host::HostController::with_demo_document([800.0, 600.0])
            .unwrap()
            .drawing()
            .unwrap();
        let second = cad_app::host::HostController::with_demo_document([800.0, 600.0])
            .unwrap()
            .drawing()
            .unwrap();
        let mut worker = NativePreparation::default();
        worker.update(input(first.clone())).unwrap();
        worker.update(input(second.clone())).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let ready = loop {
            if let Some(ready) = worker.update(input(second.clone())).unwrap() {
                break ready;
            }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        };
        assert!(!worker.current(&Some(first)));
        assert!(worker.current(&Some(second.clone())));
        let base = ready.ready.as_ref().unwrap().base.clone();
        let mut request = input(second.clone());
        request.overlays.visibility.axes = false;
        worker.update(request).unwrap();
        assert!(!worker.current(&Some(second.clone())));
        let next = loop {
            let mut request = input(second.clone());
            request.overlays.visibility.axes = false;
            if let Some(ready) = worker.update(request).unwrap() {
                break ready;
            }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        };
        assert!(Arc::ptr_eq(&base, &next.ready.as_ref().unwrap().base));
        worker.stop();
        assert!(!worker.current(&Some(second)));
    }
}
