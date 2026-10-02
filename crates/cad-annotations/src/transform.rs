//! Point, work-plane and affine-transform codecs.

use super::*;

pub(crate) fn encode_transform(t: &Transform3) -> Value {
    Value::Array(
        t.matrix
            .iter()
            .map(|row| Value::Array(row.iter().map(|v| json!(v)).collect()))
            .collect(),
    )
}

pub(crate) fn decode_transform(value: &Value) -> CadResult<Transform3> {
    let rows = value
        .as_array()
        .ok_or_else(|| corrupt("transform must be a 4x4 array"))?;
    if rows.len() != 4 {
        return Err(corrupt("transform must have 4 rows"));
    }
    let mut matrix = [[0.0f64; 4]; 4];
    for (i, row) in rows.iter().enumerate() {
        let cols = row
            .as_array()
            .ok_or_else(|| corrupt("transform row must be an array"))?;
        if cols.len() != 4 {
            return Err(corrupt("transform row must have 4 columns"));
        }
        for (j, v) in cols.iter().enumerate() {
            let n = v
                .as_f64()
                .ok_or_else(|| corrupt("transform entry must be a number"))?;
            if !n.is_finite() {
                return Err(corrupt("transform entry must be finite"));
            }
            matrix[i][j] = n;
        }
    }
    Ok(Transform3 { matrix })
}

pub(crate) fn encode_plane(plane: WorkPlane) -> Value {
    json!({
        "origin": encode_point(plane.origin),
        "u": encode_point(plane.u),
        "v": encode_point(plane.v),
    })
}

pub(crate) fn decode_plane(value: &Value) -> CadResult<WorkPlane> {
    let object = value
        .as_object()
        .ok_or_else(|| corrupt("work plane must be an object"))?;
    Ok(WorkPlane {
        origin: decode_point(require(object, "origin")?)?,
        u: decode_point(require(object, "u")?)?,
        v: decode_point(require(object, "v")?)?,
    })
}

pub(crate) fn encode_point(p: Point3) -> Value {
    json!([p.x, p.y, p.z])
}

pub(crate) fn decode_point(value: &Value) -> CadResult<Point3> {
    let a = value
        .as_array()
        .ok_or_else(|| corrupt("point must be an array"))?;
    if a.len() != 3 {
        return Err(corrupt("point must have exactly 3 coordinates"));
    }
    let read = |i: usize| {
        a[i].as_f64()
            .ok_or_else(|| corrupt("point coordinate must be a number"))
    };
    let p = Point3 {
        x: read(0)?,
        y: read(1)?,
        z: read(2)?,
    };
    if !p.x.is_finite() || !p.y.is_finite() || !p.z.is_finite() {
        return Err(corrupt("point coordinate must be finite"));
    }
    Ok(p)
}

pub(crate) fn decode_points(value: &Value) -> CadResult<Vec<Point3>> {
    value
        .as_array()
        .ok_or_else(|| corrupt("points must be an array"))?
        .iter()
        .map(decode_point)
        .collect()
}
