pub mod model;

pub use rhythm_sources::{catalog, dates, parser};

use model::{SourceDefinition, SourceGroup};

pub fn definitions() -> Vec<SourceDefinition> {
    catalog::definitions()
}
pub fn groups() -> Vec<SourceGroup> {
    catalog::groups()
}
