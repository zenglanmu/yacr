//! Unit tests.

use super::*;

#[test]
fn demo_database_has_geometry_and_bounds() {
    let controller = HostController::with_demo_document([1080.0, 1920.0]).unwrap();
    let db = controller.drawing().unwrap();
    assert!(db.entity_count() >= 6);
    let (min, max) = db.bounds().unwrap();
    assert!(max.x - min.x > 0.0);
}

fn camera_target(controller: &Rc<RefCell<HostController>>) -> Point3 {
    controller
        .borrow()
        .application
        .workspace
        .viewports
        .get(&ViewportId(1))
        .unwrap()
        .camera
        .target
}

fn world_per_px(controller: &Rc<RefCell<HostController>>) -> f64 {
    controller
        .borrow()
        .application
        .workspace
        .viewports
        .get(&ViewportId(1))
        .unwrap()
        .world_per_px()
}

fn input_harness() -> (Rc<RefCell<HostController>>, AndroidViewInput) {
    let controller = Rc::new(RefCell::new(
        HostController::with_demo_document([1080.0, 1920.0]).unwrap(),
    ));
    controller.borrow_mut().fit().unwrap();
    let input = AndroidViewInput {
        controller: controller.clone(),
        handle: Rc::new(RefCell::new(None)),
        view: Rc::new(RefCell::new(None)),
        viewport: ViewportId(1),
        last: Cell::new([0.0, 0.0]),
        dragging: Cell::new(false),
    };
    (controller, input)
}

#[test]
fn android_view_input_drag_pans_the_authoritative_camera() {
    let (controller, input) = input_harness();
    let before = camera_target(&controller);
    input.pointer(0, 1, 100.0, 100.0); // down
    input.pointer(2, 1, 140.0, 100.0); // move 40 logical px right
    input.pointer(1, 1, 140.0, 100.0); // up
    let after = camera_target(&controller);
    // Dragging right must move the camera left so the content follows the
    // finger. This is the regression that made the Android canvas inert.
    assert!(
        after.x < before.x,
        "camera did not pan: {before:?} -> {after:?}"
    );
}

#[test]
fn android_view_input_scroll_zooms_the_camera() {
    let (controller, input) = input_harness();
    let before = world_per_px(&controller);
    input.scroll(0.0, 200.0); // scroll down => factor 0.7 => zoom in
    let after = world_per_px(&controller);
    assert!(after < before, "camera did not zoom: {before} -> {after}");
}
