use cad_domain::Point3;

// --- local vector helpers (cad-spatial has no geometry prelude of its own) ---

pub(super) fn dot3(a: Point3, b: Point3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

pub(super) fn cross(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.y * b.z - a.z * b.y,
        y: a.z * b.x - a.x * b.z,
        z: a.x * b.y - a.y * b.x,
    }
}

pub(super) fn length3(v: Point3) -> f64 {
    dot3(v, v).sqrt()
}

pub(super) fn add(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x + b.x,
        y: a.y + b.y,
        z: a.z + b.z,
    }
}

pub(super) fn sub(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    }
}

pub(super) fn scale3(v: Point3, s: f64) -> Point3 {
    Point3 {
        x: v.x * s,
        y: v.y * s,
        z: v.z * s,
    }
}

pub(super) fn is_finite_point(p: Point3) -> bool {
    p.x.is_finite() && p.y.is_finite() && p.z.is_finite()
}
