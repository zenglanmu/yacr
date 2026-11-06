// Vendor properties palette: the read-only inspector a Tianzheng/plugin
// provider hands the host for the current single selection (ADR 0030 D4).
//
// It deliberately reuses the *same* surface as `PropertiesPalette`: the same
// `.palette` / `.properties__*` skeleton and the same two-column rows, so it is
// wrapped by the same `PaletteWindow` and inherits the one drag/resize/placement
// behaviour. The plugin supplies only group/row keys and verbatim values; this
// file owns the layout, the i18n lookup and the fallbacks.
//
// Everything here is display-only. There is no input, no label editor and no
// hover state that could read as "editable": in this ADR's scope every vendor
// edit is refused by the command funnel, so a fake editor would be a lie. The
// one action besides close is "Default properties", which asks the host to
// reopen the built-in palette for the same selection.
import React from 'react';
import { Icon } from '../icons.tsx';
import { Button } from './primitives.tsx';
import { useI18n } from '../context.ts';
import type { Translator } from '../i18n.ts';
import type { PluginPropertyPanel } from '../plugins/types.ts';

export interface PluginPropertiesPaletteProps {
  /** The provider's panel descriptor; already validated by the host. */
  readonly panel: PluginPropertyPanel;
  /** Close the properties dock. */
  readonly onClose: () => void;
  /** Reopen the built-in `PropertiesPalette` for the same selection. */
  readonly onShowDefault: () => void;
}

/**
 * A catalog lookup with the palette's long-standing fallback rule: an unknown
 * key is shown as itself rather than as an empty cell. The raw `group.key` is
 * the documented fallback for a group; rows use the explicit "unknown key"
 * sentence so the user can tell a missing translation from a property name.
 */
function groupLabel(key: string, t: Translator): string {
  const catalogKey = `properties.tch.group.${key}`;
  const label = t(catalogKey);
  return label === catalogKey ? key : label;
}

/** Translated class name for the header; unknown classes fall back to the raw kind. */
function kindLabel(kind: string, t: Translator): string {
  const catalogKey = `properties.tch.class.${kind}`;
  const label = t(catalogKey);
  return label === catalogKey ? kind : label;
}

function rowLabel(key: string, t: Translator): string {
  const catalogKey = `properties.tch.${key}`;
  const label = t(catalogKey);
  return label === catalogKey ? t('properties.vendor.unknown_key', { key }) : label;
}

export function PluginPropertiesPalette({ panel, onClose, onShowDefault }: PluginPropertiesPaletteProps) {
  const { t } = useI18n();
  // A group with no rows is not a section: skipping it keeps the panel honest
  // instead of printing an empty heading the plugin did not describe.
  const groups = panel.groups.filter(group => group.rows.length > 0);
  const handle = panel.handle === '' ? '\u2014' : panel.handle;
  return (
    <section
      className="palette properties plugin-properties"
      aria-label={t('properties.vendor.subtitle', { kind: kindLabel(panel.kind, t) })}
    >
      <header className="palette__head plugin-properties__head">
        <h2 className="plugin-properties__title">
          <Icon name="properties" size={15} />
          <span className="plugin-properties__kind" title={panel.kind}>
            {kindLabel(panel.kind, t)}
          </span>
          <span className="plugin-properties__badge">{t('properties.vendor.title')}</span>
        </h2>
      </header>

      <div className="palette__body plugin-properties__body">
        {/* Provenance, not identity: the handle says where the entity came from
            (ADR 0030 D1). The vendor subtitle names the decoded type for screen
            readers on the section label above; it would only repeat `kind` here. */}
        <div className="plugin-properties__source">
          <p className="plugin-properties__handle">
            {t('properties.vendor.handle', { handle })}
          </p>
        </div>

        {groups.length > 0 ? (
          <div className="properties__groups">
            {groups.map(group => (
              <section
                key={group.key}
                className="properties__group plugin-properties__group"
                aria-labelledby={`plugin-properties-group-${group.key}`}
              >
                <h3 id={`plugin-properties-group-${group.key}`} className="properties__group-title">
                  {groupLabel(group.key, t)}
                </h3>
                <dl className="properties__rows">
                  {group.rows.map((row, index) => (
                    <div
                      key={`${row.key}:${index}`}
                      className={[
                        'properties__row',
                        'plugin-properties__row',
                        row.key === 'text.content' ? 'is-text' : '',
                      ].filter(Boolean).join(' ')}
                    >
                      <dt title={row.key}>{rowLabel(row.key, t)}</dt>
                      <dd>
                        <span className="properties__readonly plugin-properties__value">
                          {row.value}
                        </span>
                      </dd>
                    </div>
                  ))}
                </dl>
              </section>
            ))}
          </div>
        ) : null}
      </div>

      <footer className="plugin-properties__footer">
        <Button
          icon="properties"
          iconSize={14}
          onClick={onShowDefault}
          aria-label={t('properties.vendor.show_default')}
        >
          {t('properties.vendor.show_default')}
        </Button>
        <Button
          icon="close"
          iconSize={14}
          onClick={onClose}
          aria-label={t('properties.vendor.close')}
        >
          {t('properties.vendor.close')}
        </Button>
      </footer>
    </section>
  );
}
