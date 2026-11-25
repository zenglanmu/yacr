// The mechanical profile's left-hand tray (SolidWorks FeatureManager-derived
// panel list).
//
// SolidWorks docks the FeatureManager design tree, the PropertyManager and the
// ConfigurationManager on the **left**, which is why the profile's `traySide` is
// `left` (see `domains.ts`). Only panels with a real body are live:
//
//   * the FeatureManager design tree reads the host's `cad_mechanical_tree`
//     projection (features in evaluation order, then bodies);
//   * PropertyManager reuses the shared entity-info body over the real
//     `cad_status.selection` rows and points at the full Properties palette;
//   * Instructor names the active tool and its real next step.
//
// The ConfigurationManager has no body in this build (configurations exist in
// the file format, but there is no create/switch UI), so it stays an explicit
// `pending` panel with its own sentence instead of an empty box.
import type { TrayPanel } from '../ui/slots.ts';

export const MECHANICAL_TRAY: readonly TrayPanel[] = Object.freeze<TrayPanel[]>([
  { id: 'mechanical.tree', titleKey: 'mechanical.panel.tree', kind: 'mechanicalTree' },
  { id: 'mechanical.properties', titleKey: 'mechanical.panel.properties', kind: 'entityInfo' },
  {
    id: 'mechanical.configurations',
    titleKey: 'mechanical.panel.configurations',
    kind: 'pending',
    pendingKey: 'mechanical.pending.configurations',
  },
  { id: 'mechanical.instructor', titleKey: 'mechanical.panel.instructor', kind: 'instructor' },
]);
