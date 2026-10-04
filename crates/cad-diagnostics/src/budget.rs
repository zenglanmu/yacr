//! Resource accounting shared by importers, caches and the CLI (spec §8.4,
//! §11.2, §19).
//!
//! Spec §11.2 requires that decoded data is bounded by record/vertex/recursion
//! limits *and* that the limits are written into diagnostics — "不能静默截断后
//! 报告完整" (must not silently truncate and then report complete). The audit
//! (F10) found that budgets were declared but never wired. This module makes a
//! budget an explicit object that either accepts a charge or returns an
//! over-budget reason, so callers cannot silently drop the excess.
//!
//! Accounting is intentionally integer-only and free of platform types so the
//! same logic runs in the native host, Wasm and CLI.

use cad_domain::ObjectId;

use crate::model::{codes, DiagnosticParameter, DiagnosticReason};

/// A category of resource or derived data with its own sub-budget.
///
/// Codes are stable schema keys, not localized text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BudgetCategory {
    FileBytes,
    DomainBytes,
    CpuGeometryBytes,
    GpuEstimatedBytes,
    AtlasBytes,
    AttachmentBytes,
    FontCacheBytes,
    ImagePixels,
    ProxyRecords,
    ProxyVertices,
}

impl BudgetCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            BudgetCategory::FileBytes => "file_bytes",
            BudgetCategory::DomainBytes => "domain_bytes",
            BudgetCategory::CpuGeometryBytes => "cpu_geometry_bytes",
            BudgetCategory::GpuEstimatedBytes => "gpu_estimated_bytes",
            BudgetCategory::AtlasBytes => "atlas_bytes",
            BudgetCategory::AttachmentBytes => "attachment_bytes",
            BudgetCategory::FontCacheBytes => "font_cache_bytes",
            BudgetCategory::ImagePixels => "image_pixels",
            BudgetCategory::ProxyRecords => "proxy_records",
            BudgetCategory::ProxyVertices => "proxy_vertices",
        }
    }
}

/// A total budget plus optional per-category caps.
///
/// `total` is the hard ceiling for all charged categories; `per_category`
/// additionally caps individual categories (for example the font cache).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceBudget {
    pub total: u64,
    pub per_category: Vec<(BudgetCategory, u64)>,
    pub max_recursion_depth: u32,
    used_total: u64,
    used_by_category: Vec<(BudgetCategory, u64)>,
}

impl Default for ResourceBudget {
    fn default() -> Self {
        ResourceBudget {
            total: 256 * 1024 * 1024,
            per_category: Vec::new(),
            max_recursion_depth: 16,
            used_total: 0,
            used_by_category: Vec::new(),
        }
    }
}

impl ResourceBudget {
    pub fn new(total: u64, max_recursion_depth: u32) -> Self {
        ResourceBudget {
            total,
            per_category: Vec::new(),
            max_recursion_depth,
            used_total: 0,
            used_by_category: Vec::new(),
        }
    }

    pub fn with_category(mut self, category: BudgetCategory, cap: u64) -> Self {
        self.per_category.push((category, cap));
        self
    }

    pub fn total(&self) -> u64 {
        self.total
    }

    pub fn used(&self) -> u64 {
        self.used_total
    }

    pub fn used_in(&self, category: BudgetCategory) -> u64 {
        self.used_by_category
            .iter()
            .find(|(c, _)| *c == category)
            .map(|(_, used)| *used)
            .unwrap_or(0)
    }

    pub fn cap_for(&self, category: BudgetCategory) -> Option<u64> {
        self.per_category
            .iter()
            .find(|(c, _)| *c == category)
            .map(|(_, cap)| *cap)
    }

    /// Charge `amount` units against `category`, or return an over-budget reason.
    ///
    /// The charge is all-or-nothing: on rejection nothing is recorded, so the
    /// caller knows the exact shortfall instead of a partially applied budget.
    /// Unrepresentable totals are rejected even at `u64::MAX` limits; the
    /// requested count in the diagnostic saturates at `u64::MAX`.
    pub fn charge(
        &mut self,
        object: Option<ObjectId>,
        category: BudgetCategory,
        amount: u64,
    ) -> Result<(), DiagnosticReason> {
        let used_in_category = self.used_in(category);
        let Some(next_category) = used_in_category.checked_add(amount) else {
            return Err(self.over_budget(
                category,
                object,
                used_in_category,
                amount,
                self.cap_for(category).unwrap_or(self.total),
            ));
        };
        if let Some(cap) = self.cap_for(category) {
            if next_category > cap {
                return Err(self.over_budget(category, object, used_in_category, amount, cap));
            }
        }
        let Some(next_total) = self.used_total.checked_add(amount) else {
            return Err(self.over_budget(category, object, self.used_total, amount, self.total));
        };
        if next_total > self.total {
            return Err(self.over_budget(category, object, self.used_total, amount, self.total));
        }
        self.used_total = next_total;
        match self
            .used_by_category
            .iter_mut()
            .find(|(c, _)| *c == category)
        {
            Some((_, used)) => *used = next_category,
            None => self.used_by_category.push((category, amount)),
        }
        Ok(())
    }

    /// Guard a recursion step: returns an error once the depth is exceeded.
    pub fn check_depth(
        &self,
        object: Option<ObjectId>,
        depth: u32,
    ) -> Result<(), DiagnosticReason> {
        if depth > self.max_recursion_depth {
            let mut parameters = Vec::new();
            if let Some(id) = object {
                parameters.push(DiagnosticParameter::Object(id));
            }
            parameters.push(DiagnosticParameter::Depth(depth as u64));
            parameters.push(DiagnosticParameter::Limit(self.max_recursion_depth as u64));
            return Err(DiagnosticReason::missing(
                codes::RESOURCE_RECURSION_LIMIT,
                parameters,
            ));
        }
        Ok(())
    }

    fn over_budget(
        &self,
        category: BudgetCategory,
        object: Option<ObjectId>,
        used: u64,
        amount: u64,
        limit: u64,
    ) -> DiagnosticReason {
        let mut parameters = Vec::new();
        if let Some(id) = object {
            parameters.push(DiagnosticParameter::Object(id));
        }
        parameters.push(DiagnosticParameter::Identifier(
            category.as_str().to_string(),
        ));
        parameters.push(DiagnosticParameter::Bytes(amount));
        parameters.push(DiagnosticParameter::Limit(limit));
        parameters.push(DiagnosticParameter::Count(used.saturating_add(amount)));
        DiagnosticReason::missing(codes::RESOURCE_OVER_BUDGET, parameters)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn total_budget_rejects_rather_than_dropping() {
        let mut budget = ResourceBudget::new(100, 4);
        budget
            .charge(None, BudgetCategory::FontCacheBytes, 60)
            .unwrap();
        let reason = budget
            .charge(None, BudgetCategory::FontCacheBytes, 60)
            .unwrap_err();
        assert_eq!(reason.code, codes::RESOURCE_OVER_BUDGET);
        // The rejected charge did not apply.
        assert_eq!(budget.used(), 60);
        // The reason is structured, not a prose string.
        assert!(reason
            .parameters
            .iter()
            .any(|p| matches!(p, DiagnosticParameter::Limit(100))));
    }

    #[test]
    fn per_category_cap_is_enforced() {
        let mut budget =
            ResourceBudget::new(1000, 4).with_category(BudgetCategory::ImagePixels, 50);
        budget
            .charge(None, BudgetCategory::ImagePixels, 50)
            .unwrap();
        let reason = budget
            .charge(None, BudgetCategory::ImagePixels, 1)
            .unwrap_err();
        assert_eq!(reason.code, codes::RESOURCE_OVER_BUDGET);
        // A different category is unaffected by the image cap.
        budget
            .charge(None, BudgetCategory::CpuGeometryBytes, 200)
            .unwrap();
    }

    #[test]
    fn recursion_depth_is_enforced() {
        let budget = ResourceBudget::new(1000, 2);
        budget.check_depth(None, 2).unwrap();
        let reason = budget.check_depth(Some(ObjectId(7)), 3).unwrap_err();
        assert_eq!(reason.code, codes::RESOURCE_RECURSION_LIMIT);
        assert!(reason
            .parameters
            .contains(&DiagnosticParameter::Object(ObjectId(7))));
    }

    #[test]
    fn maximum_category_limit_still_rejects_counter_overflow_atomically() {
        let category = BudgetCategory::FontCacheBytes;
        let mut budget = ResourceBudget::new(u64::MAX, 4).with_category(category, u64::MAX);
        budget.charge(None, category, u64::MAX).unwrap();
        let before = budget.clone();
        let reason = budget.charge(Some(ObjectId(7)), category, 1).unwrap_err();
        assert_eq!(reason.code, codes::RESOURCE_OVER_BUDGET);
        assert!(reason
            .parameters
            .contains(&DiagnosticParameter::Count(u64::MAX)));
        assert!(reason
            .parameters
            .contains(&DiagnosticParameter::Object(ObjectId(7))));
        assert_eq!(budget, before);
        budget.charge(None, category, 0).unwrap();
        assert_eq!(budget, before);
    }

    #[test]
    fn maximum_total_limit_rejects_overflow_across_categories() {
        let mut budget = ResourceBudget::new(u64::MAX, 4);
        budget
            .charge(None, BudgetCategory::FileBytes, u64::MAX - 1)
            .unwrap();
        budget
            .charge(None, BudgetCategory::FontCacheBytes, 1)
            .unwrap();
        let before = budget.clone();
        let reason = budget
            .charge(None, BudgetCategory::ImagePixels, 1)
            .unwrap_err();
        assert_eq!(reason.code, codes::RESOURCE_OVER_BUDGET);
        assert_eq!(budget, before);
        assert_eq!(budget.used_in(BudgetCategory::ImagePixels), 0);
    }
}
