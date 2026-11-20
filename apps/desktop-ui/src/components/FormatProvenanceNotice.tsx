// The open-time format provenance notice (ADR-0041 D5).
//
// A document opened from a format whose specification is not published says so
// once, when it opens: the reader is an independent reconstruction, exporting
// back may not be accepted by the originating application, and some information
// is not modelled here. That is news about the format, not a failure of the
// open - so this is the `UpdateNotice` pattern, never the modal host notice:
//
//   * an `aside` in a pointer-transparent layer docked above the status bar
//     (`Workspace` mounts it inside the workspace, so the native CAD surface
//     rect never changes);
//   * `role="status"` + `aria-live="polite"`: announced, never interrupting;
//   * no backdrop, no focus trap, no global key handler, and the card itself is
//     not focusable - Escape still belongs to the CAD session, and the only
//     focusable node is the real dismiss button;
//   * the copy is entirely `cad/opened-source.ts`; this file only lays it out.
import React, { useId } from 'react';
import { Icon } from '../icons.tsx';
import { Button } from './primitives.tsx';
import { useI18n } from '../context.ts';
import { openNoticeView } from '../cad/opened-source.ts';
import type { CadOpenedSource } from '../cad/opened-source.ts';

export interface FormatProvenanceNoticeProps {
  /** The validated `cad_status.openedSource` block for the current document. */
  readonly source: CadOpenedSource;
  /** Hide the notice for this open (the host keys it on the document). */
  readonly onDismiss: () => void;
}

export function FormatProvenanceNotice({ source, onDismiss }: FormatProvenanceNoticeProps) {
  const { t } = useI18n();
  const titleId = useId();
  const view = openNoticeView(source, t);
  const dismissLabel = t('open.notice.dismiss');
  return (
    <aside
      className="format-notice"
      role="status"
      aria-live="polite"
      aria-labelledby={titleId}
    >
      <span className="format-notice__mark" aria-hidden="true">
        <Icon name="info" size={18} />
      </span>
      <div className="format-notice__content">
        <p id={titleId} className="format-notice__title">{view.title}</p>
        {view.lines.map(line => (
          <p
            key={line.key}
            className={line.key === 'ymodel'
              ? 'format-notice__body is-recommend'
              : 'format-notice__body'}
          >
            {line.text}
          </p>
        ))}
      </div>
      <div className="format-notice__actions">
        {/* A real button, not a clickable card: the card stays unfocusable and
            carries no dismiss handler of its own. */}
        <Button variant="chip" size="sm" aria-label={dismissLabel} onClick={onDismiss}>
          {dismissLabel}
        </Button>
      </div>
    </aside>
  );
}
