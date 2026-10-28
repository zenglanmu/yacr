// The mobile ribbon: a drawer that drops from the top bar, not a 92px band.
//
// It renders the **same model** the desktop band renders (`RIBBON_TABS`), through
// the same carrier reduction (`layout/mobile-ribbon.ts`) and the same dispatch
// and refusal helpers the band uses (`runItem`, `itemEnabled`, `unavailable` from
// `Ribbon.tsx`). Nothing here re-implements admission, so a tool that is disabled
// in the band is disabled in the drawer, and running one reaches the host by the
// identical path.
//
// Shape, and why it differs from the band:
//
//   * **Tab chips, not a tab strip.** The seven tabs cannot share 390px, so the
//     chip row scrolls horizontally and every chip is a 44px target.
//   * **A two-or-more column touch grid, not a 92px band.** Each cell is an icon,
//     its label and — while it cannot run — the same refusal sentence the band
//     shows as a tooltip. A phone has no hover, so the reason is text.
//   * **Variants are disclosed per item.** The band opens a caret popover that
//     cannot cross the band; here the item's options expand underneath it as
//     full-width rows, and each row keeps its own gate and its own refusal.
//
// The drawer is an in-DOM overlay in this same tree (no second window, no
// portal). It drops over the drawing and reflows nothing: the CAD partition's
// rect is untouched, which is what keeps the host's surface metrics valid.
import React, { useEffect, useMemo, useRef, useState } from 'react';
import { Icon } from '../icons.tsx';
import type { IconName } from '../icons.tsx';
import { IconButton } from './primitives.tsx';
import { itemEnabled, runItem, unavailable } from './Ribbon.tsx';
import { useI18n, useShell } from '../context.ts';
import type { ShellValue } from '../context.ts';
import type { Translator } from '../i18n.ts';
import { ribbonTab } from '../ribbon/ribbon-model.ts';
import type { RibbonItem } from '../ribbon/ribbon-model.ts';
import { mobileRibbonDomId, mobileRibbonSections, mobileRibbonTabs, panelRefusalKey, sharedRefusal } from '../layout/mobile-ribbon.ts';

/** The drawer's DOM id: the top bar's `aria-controls` names it. */
export const MOBILE_DRAWER_ID = 'cad-mobile-drawer';

export interface MobileRibbonDrawerProps {
  readonly open: boolean;
  /** Close the drawer (backdrop, close button, Escape, after running an item). */
  readonly onClose: () => void;
}

/** A disabled item's visible reason, in the band's own words. */
interface DrawerReasonProps {
  readonly item: RibbonItem;
}

function DrawerReason({ item }: DrawerReasonProps) {
  const { t } = useI18n();
  const shell = useShell();
  return <p className="mobile-drawer__reason">{unavailable(t, item, shell)}</p>;
}

/**
 * `unavailable` with a translator that answers its own key, so the refusal's
 * **key** can be read through the same three branches the visible sentence goes
 * through. Never rendered: `unavailable` composes `t(reasonKey, {command})`, and
 * this hands the `reasonKey` back instead of a sentence.
 */
const keyOnly = ((key: string) => key) as Translator;

/** Whether a value read through `keyOnly` really is a catalog key. */
function looksLikeKey(value: string): boolean {
  return /^[a-z][a-z0-9_]*(\.[a-z0-9_]+)*$/.test(value);
}

/** One item's refusal reason key, or `null` while it is enabled. */
function refusalKey(shell: ShellValue, item: RibbonItem): string | null {
  if (itemEnabled(shell, item)) return null;
  const key = unavailable(keyOnly, item, shell);
  // If `unavailable` ever stops answering a catalog key (someone builds a
  // sentence inline), the reasons are not comparable: answer `null` so the panel
  // keeps the per-cell copy instead of printing the raw value.
  return looksLikeKey(key) ? key : null;
}

/**
 * The refusal a whole panel shares, in **command-free** panel words, or `null`
 * when the cells must speak for themselves (see `sharedRefusal`).
 *
 * The comparison is by reason key, not by sentence: the sentence names the
 * command it belongs to ("Command `COPY` is not supported yet."), so on a
 * carrier whose vocabulary is empty every cell used to read differently and
 * nothing ever collapsed.
 */
function sectionRefusal(shell: ShellValue, t: Translator, items: readonly RibbonItem[]): string | null {
  const noteKey = panelRefusalKey(sharedRefusal(items.map(item => refusalKey(shell, item))));
  return noteKey === null ? null : t(noteKey);
}

interface DrawerVariantProps {
  readonly item: RibbonItem;
  readonly variant: RibbonItem;
  readonly onClose: () => void;
}

function DrawerVariant({ item, variant, onClose }: DrawerVariantProps) {
  const { t } = useI18n();
  const shell = useShell();
  const enabled = itemEnabled(shell, variant);
  return (
    <li className="mobile-drawer__variant-row">
      <button
        type="button"
        className="mobile-drawer__variant"
        disabled={!enabled}
        title={enabled && variant.hintKey ? t(variant.hintKey) : undefined}
        onClick={enabled ? () => { runItem(shell, variant); onClose(); } : undefined}
      >
        <Icon name={(variant.icon ?? item.icon) as IconName} size={16} />
        <span className="mobile-drawer__variant-label">{t(variant.labelKey)}</span>
        {enabled ? null : <em className="mobile-drawer__badge">{t('command.badge.unwired')}</em>}
      </button>
      {enabled ? null : <DrawerReason item={variant} />}
    </li>
  );
}

interface DrawerItemProps {
  readonly item: RibbonItem;
  readonly openMenu: string | null;
  readonly setOpenMenu: (id: string | null) => void;
  readonly onClose: () => void;
  /** Whether this cell prints its own refusal (false when the panel shares one). */
  readonly showReason: boolean;
}

function DrawerItem({ item, openMenu, setOpenMenu, onClose, showReason }: DrawerItemProps) {
  const { t } = useI18n();
  const shell = useShell();
  const enabled = itemEnabled(shell, item);
  const variants = item.variants ?? [];
  const menuOpen = openMenu === item.id;
  const optionsId = mobileRibbonDomId(item.id);
  return (
    <div className={['mobile-drawer__cell', enabled ? '' : 'is-disabled', menuOpen ? 'is-open' : ''].filter(Boolean).join(' ')}>
      <button
        type="button"
        className="mobile-drawer__item"
        disabled={!enabled}
        title={enabled && item.hintKey ? t(item.hintKey) : undefined}
        onClick={enabled ? () => { runItem(shell, item); onClose(); } : undefined}
      >
        <Icon name={item.icon as IconName} size={22} />
        <span className="mobile-drawer__label">{t(item.labelKey)}</span>
      </button>
      {enabled || !showReason ? null : <DrawerReason item={item} />}
      {variants.length > 0 ? (
        <>
          <button
            type="button"
            className="mobile-drawer__variants-toggle"
            aria-expanded={menuOpen}
            aria-controls={menuOpen ? optionsId : undefined}
            onClick={() => setOpenMenu(menuOpen ? null : item.id)}
          >
            <Icon name={menuOpen ? 'chevronUp' : 'chevronDown'} size={14} />
            <span>{t('ribbon.variants', { name: t(item.labelKey) })}</span>
          </button>
          {menuOpen ? (
            <ul className="mobile-drawer__variants" id={optionsId}>
              {variants.map(variant => (
                <DrawerVariant key={variant.id} item={item} variant={variant} onClose={onClose} />
              ))}
            </ul>
          ) : null}
        </>
      ) : null}
    </div>
  );
}

export function MobileRibbonDrawer({ open, onClose }: MobileRibbonDrawerProps) {
  const { t } = useI18n();
  const shell = useShell();
  const drawerRef = useRef<HTMLDivElement | null>(null);
  const [openMenu, setOpenMenu] = useState<string | null>(null);
  const tab = ribbonTab(shell.ribbonTab);
  // `false`: the compact layout never renders the docked palettes, so the drawer
  // drops their toggles with the panels — the same carrier rule a `dom-canvas`
  // host uses, and the reason no button here can open a surface that is hidden.
  const sections = useMemo(() => mobileRibbonSections(shell.ribbonTab, false), [shell.ribbonTab]);

  // Escape closes the drawer and nothing else: the shell's global handler is
  // left alone (it also clears history/diagnostics on Escape, which is harmless
  // here and keeps the two handlers from disagreeing about ownership).
  useEffect(() => {
    if (!open) return undefined;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || event.defaultPrevented) return;
      // An IME composition owns Escape (it cancels the composition).
      if (event.isComposing || event.keyCode === 229) return;
      event.preventDefault();
      onClose();
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [open, onClose]);

  // An open drawer takes focus, so a screen reader enters the dialog and the
  // command line's field is blurred (which also resumes the CAD pointer lane).
  useEffect(() => {
    if (open) drawerRef.current?.focus?.({ preventScroll: true });
  }, [open]);

  // A collapsed drawer keeps no expanded option list for the next open.
  useEffect(() => {
    if (!open) setOpenMenu(null);
  }, [open]);

  if (!open) return null;

  return (
    <div className="mobile-drawer-layer">
      <div
        id={MOBILE_DRAWER_ID}
        ref={drawerRef}
        className="mobile-drawer"
        role="dialog"
        aria-modal="true"
        aria-label={t('mobile.drawer')}
        tabIndex={-1}
      >
        <div className="mobile-drawer__head">
          <span className="mobile-drawer__title">{t('mobile.drawer')}</span>
          <IconButton icon="close" label={t('mobile.close')} onClick={onClose} />
        </div>
        <div className="mobile-drawer__tabs" role="tablist" aria-label={t('ribbon.tabs_label')}>
          {mobileRibbonTabs().map(entry => (
            <button
              key={entry.id}
              type="button"
              role="tab"
              aria-selected={entry.id === shell.ribbonTab}
              className={[
                'mobile-drawer__tab',
                entry.accent ? 'is-accent' : '',
                entry.id === shell.ribbonTab ? 'is-active' : '',
              ].filter(Boolean).join(' ')}
              onClick={() => { setOpenMenu(null); shell.setRibbonTab(entry.id); }}
            >
              {t(entry.labelKey)}
            </button>
          ))}
        </div>
        <div className="mobile-drawer__body" role="tabpanel" aria-label={t(tab.labelKey)}>
          {sections.map(section => {
            const note = sectionRefusal(shell, t, section.items);
            return (
              <section key={section.id} className="mobile-drawer__section" aria-label={t(section.titleKey)}>
                <h3 className="mobile-drawer__section-title">{t(section.titleKey)}</h3>
                {note ? <p className="mobile-drawer__note">{note}</p> : null}
                <div className="mobile-drawer__grid">
                  {section.items.map(item => (
                    <DrawerItem
                      key={item.id}
                      item={item}
                      openMenu={openMenu}
                      setOpenMenu={setOpenMenu}
                      onClose={onClose}
                      showReason={note === null}
                    />
                  ))}
                </div>
              </section>
            );
          })}
        </div>
      </div>
      {/* The backdrop is a real button: a tap outside closes the drawer, and its
          accessible name says what the tap does. It sits below the panel, so it
          never covers the chips or the grid. */}
      <button type="button" className="mobile-drawer__backdrop" aria-label={t('mobile.close')} onClick={onClose} />
    </div>
  );
}
