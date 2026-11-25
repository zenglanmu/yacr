// The BIM profile's right-hand tray (Revit-derived panel list).
//
// Revit docks Properties and the Project Browser on the left; this chrome has
// one tray slot, which sits on the right. The placement is an approximation and
// is declared as such: the panel **order and meaning** follow the reference,
// the side does not.
//
// Only panels with a real body are live:
//
//   * Properties reads the host's `cad_status.selection` rows and points at the
//     full Properties palette (the same body the concept tray uses);
//   * Visibility/Graphics opens the existing Layers palette, which owns the real
//     visibility state;
//   * BIM Browse opens the read-only component panel over `cad_bim_query`;
//   * Instructor names the active tool and its real next step.
//
// Project Browser and Schedules have no data source in this build (there is no
// view/sheet tree and no Revit schedule model), so they stay explicit `pending`
// panels with their own sentence instead of empty boxes.
import type { TrayPanel } from '../ui/slots.ts';

const pending = (id: string, titleKey: string, pendingKey: string): TrayPanel => ({
  id,
  titleKey,
  kind: 'pending',
  pendingKey,
});

export const BIM_TRAY: readonly TrayPanel[] = Object.freeze<TrayPanel[]>([
  { id: 'bim.properties', titleKey: 'bim.panel.properties', kind: 'entityInfo' },
  pending('bim.project_browser', 'bim.panel.project_browser', 'bim.pending.project_browser'),
  { id: 'bim.visibility', titleKey: 'bim.panel.visibility', kind: 'tags' },
  { id: 'bim.browse', titleKey: 'bim.panel.browse', kind: 'bimQuery' },
  pending('bim.schedule_view', 'bim.panel.schedules', 'bim.pending.schedules'),
  { id: 'bim.instructor', titleKey: 'bim.panel.instructor', kind: 'instructor' },
]);
