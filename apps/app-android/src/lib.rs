//! Android assembly slot; no android_main, SAF or GPU integration yet.
use cad_domain::*;
pub struct AndroidHostConfiguration { pub recovery_enabled: bool }
pub fn start(_configuration: AndroidHostConfiguration) -> CadResult<()> { pending("host.android.activity_saf_composition") }
