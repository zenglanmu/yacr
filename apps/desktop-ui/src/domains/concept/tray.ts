// The SketchUp default tray, in the official panel order from the "Dialog
// Boxes and Trays" page. Panels whose body this build actually has are real;
// the rest are explicit `pending` panels with their own sentence. The shipped
// default-tray subset is not published by the reference docs, so this list is
// the full documented panel list: a panel we cannot render still keeps its name
// and place, closed by default, and never claims a body it does not have.
import type { TrayPanel } from '../ui/slots.ts';

const pending = (id: string, titleKey: string): TrayPanel => ({
  id,
  titleKey,
  kind: 'pending',
  pendingKey: 'concept.pending.panel',
});

export const CONCEPT_TRAY: readonly TrayPanel[] = Object.freeze<TrayPanel[]>([
  { id: 'concept.entity_info', titleKey: 'concept.panel.entity_info', kind: 'entityInfo' },
  { id: 'concept.materials', titleKey: 'concept.panel.materials', kind: 'materials' },
  { id: 'concept.components', titleKey: 'concept.panel.components', kind: 'components' },
  { id: 'concept.styles', titleKey: 'concept.panel.styles', kind: 'styles' },
  { id: 'concept.tags', titleKey: 'concept.panel.tags', kind: 'tags' },
  { id: 'concept.scenes', titleKey: 'concept.panel.scenes', kind: 'scenes' },
  { id: 'concept.shadows', titleKey: 'concept.panel.shadows', kind: 'shadows' },
  { id: 'concept.fog', titleKey: 'concept.panel.fog', kind: 'fog' },
  pending('concept.match_photo', 'concept.panel.match_photo'),
  { id: 'concept.soften_edges', titleKey: 'concept.panel.soften_edges', kind: 'soften' },
  pending('concept.overlays', 'concept.panel.overlays'),
  { id: 'concept.instructor', titleKey: 'concept.panel.instructor', kind: 'instructor' },
  { id: 'concept.outliner', titleKey: 'concept.panel.outliner', kind: 'outliner' },
]);
