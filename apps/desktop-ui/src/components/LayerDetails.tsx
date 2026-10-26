// The row's details line: the color control, the four flags as engaged-state
// toggles, and the three read-only attributes.
//
// It is a component of its own for two reasons, and neither is decoration:
//
//  * **It only exists while it is open.** The palette owns that state (the dot, the
//    chevron and the "more states" chip all open the same line for one row at a
//    time), so a row that is not open keeps the 28px height the palette always had.
//    Nothing here reserves space.
//  * **Its job is narrow: render what the row says, send one write per press.**
//    Every value comes from the row (`layer-attributes.ts` decides which), every
//    press goes back through the callbacks the palette hands in (the store's
//    single-flight path), and every sentence is catalog copy — including what a
//    flag *actually* does today (`layers.<flag>.hint`: off/frozen hide the layer
//    through the shared visibility path, locked/plottable are stored only).
//
// Two deliberate choices:
//
//  * **One write per colour pick, on `change`.** A native `<input type="color">`
//    emits live `input` frames while the picker is open; writing on those would
//    queue a layer-table write per frame and fight the single-flight window. A
//    committed `change` is one write.
//  * **Escape closes it, and it stops the event.** The panel catches its own Escape
//    (so the row's form handler never sees a second one) and the palette restores
//    focus to the trigger that opened it.
import React from 'react';
import { IconButton } from './primitives.tsx';
import { useI18n } from '../context.ts';
import { layerAttributes, layerFlags } from '../layers/layer-attributes.ts';
import type { LayerRow } from '../layers/layers.ts';

/** What the panel needs from the row, and the three things a press may do. */
export interface LayerDetailsProps {
  /** The row the line belongs to (its stored attributes are what is shown). */
  readonly layer: LayerRow;
  /** The `#rrggbb` the colour control starts at (`layerSwatch().color`). */
  readonly color: string;
  /** Whether `color` is the drawing's own (false: the name-derived marker). */
  readonly fromColor: boolean;
  /** The id the colour hint is published under (`aria-describedby`). */
  readonly hintId: string;
  /** A mutation is in flight: presses are dropped, and the line reads as busy. */
  readonly busy: boolean;
  readonly onPickColor: (value: string) => void;
  readonly onToggleFlag: (key: string, value: string) => void;
  readonly onClose: () => void;
}

export function LayerDetails({
  layer,
  color,
  fromColor,
  hintId,
  busy,
  onPickColor,
  onToggleFlag,
  onClose,
}: LayerDetailsProps) {
  const { t } = useI18n();
  return (
    <div
      className="layers__details"
      role="group"
      aria-label={t('layers.attributes')}
      // The panel's own Escape: closing here keeps the row's form handler from
      // receiving an event it would treat as a second cancel.
      onKeyDown={event => {
        if (event.key !== 'Escape' || event.defaultPrevented) return;
        event.preventDefault();
        event.stopPropagation();
        onClose();
      }}
    >
      <div className="layers__details-head">
        <span className="layers__field-label">{t('layers.color')}</span>
        <input
          type="color"
          className="layers__color-input"
          value={color}
          aria-label={t('layers.color')}
          aria-describedby={hintId}
          onChange={event => onPickColor(event.target.value)}
        />
        {layer.color.rgb ? (
          <span className="layers__attr-value">{layer.color.rgb}</span>
        ) : null}
        {layer.color.aci !== null ? (
          <span className="layers__attr-value">
            {t('layers.color.aci', { index: layer.color.aci })}
          </span>
        ) : null}
        <IconButton
          icon="close"
          size="xs"
          label={t('layers.attributes.close')}
          onClick={onClose}
        />
      </div>
      {/* What a pick writes, in the control's own words: a true colour replaces the
          layer's index colour, and the drawing is not recoloured by it yet. */}
      <p className="layers__hint" id={hintId}>{t('layers.color.hint')}</p>
      {/* The dot needing a fallback is said in words too: a picker that starts at
          the marker's tone must not look like it shows the drawing's stored color. */}
      {fromColor ? null : (
        <p className="layers__note">{t('layers.color.marker')}</p>
      )}
      {/* The four flags, as toggles in both directions: `aria-pressed` is the row's
          own state, and a press sends its opposite. The hint carries the honest
          behavior, so the panel never implies a freeze/lock/plot rule that is not
          implemented. */}
      <div className="layers__flags">
        {layerFlags(layer).map(flag => (
          <button
            key={flag.key}
            type="button"
            className="layers__chip layers__chip--action"
            aria-pressed={flag.value}
            aria-disabled={busy || undefined}
            title={t(flag.hintKey)}
            onClick={() => onToggleFlag(flag.key, flag.next)}
          >
            {t(flag.labelKey)}
          </button>
        ))}
      </div>
      {/* Read-only, verbatim: the drawing's own stored form (no unit is invented
          and nothing is re-scaled). */}
      <p className="layers__attrs">
        {layerAttributes(layer).map(attribute => (
          <span key={attribute.key} className="layers__attr">
            <span className="layers__attr-label">{t(attribute.labelKey)}</span>
            <span className="layers__attr-value">{attribute.value}</span>
          </span>
        ))}
      </p>
      <p className="layers__hint">{t('layers.attributes.hint')}</p>
    </div>
  );
}
