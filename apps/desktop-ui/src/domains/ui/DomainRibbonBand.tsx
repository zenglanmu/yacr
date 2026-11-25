// A domain-owned ribbon band.
//
// `bim` (the Revit-style profile) does not reuse the AutoCAD tab model: its tabs
// and panels come from the domain slot registry (`DomainUiLayout.ribbon`). The
// band itself is the same chrome region and the same panel renderer the drafting
// ribbon uses (`RibbonPanelView`), so a domain item keeps the shared admission
// (`itemEnabled`/`unavailable`/`runItem`), the shared variant popover and the
// shared overflow cues. Nothing here is a second dispatch path.
//
// The active tab is **local** to the band. It is presentation state, not a
// document or router fact: the drafting `shell.ribbonTab` belongs to the AutoCAD
// tab table, and writing a Revit tab id into it would make the drafting ribbon
// and the mobile drawer disagree with what is on screen. Collapsing the band is
// also not offered: the drafting collapse control swaps in the AutoLISP-style
// menubar, which has no meaning for this profile.
import React, { useEffect, useRef, useState } from 'react';
import { useI18n, useShell } from '../../context.ts';
import {
  OverflowCue,
  RibbonPanelView,
  pageRibbon,
  useRibbonOverflow,
} from '../../components/Ribbon.tsx';
import { offeredPanels, tabLabel } from '../../ribbon/ribbon-model.ts';
import { ribbonCueReserve } from '../../ribbon/ribbon-overflow.ts';
import type { RibbonTab } from '../../ribbon/ribbon-model.ts';

export interface DomainRibbonBandProps {
  readonly tabs: readonly RibbonTab[];
}

export function DomainRibbonBand({ tabs }: DomainRibbonBandProps) {
  const { t, locale } = useI18n();
  const shell = useShell();
  // Open on the first non-accented tab. The accented tab is the reference's
  // File/application menu, and the drafting ribbon likewise opens on `home`
  // rather than `file`: a band whose default view is a file menu hides the work.
  const [activeId, setActiveId] = useState(() => (tabs.find(entry => !entry.accent) ?? tabs[0])?.id ?? '');
  const [openMenu, setOpenMenu] = useState<string | null>(null);
  const panelsRef = useRef<HTMLDivElement | null>(null);

  // Re-registering the domain layout (or a plugin patch) can remove the active
  // tab; fall back to the first one instead of rendering an empty band.
  useEffect(() => {
    if (!tabs.some(entry => entry.id === activeId)) setActiveId(tabs[0]?.id ?? '');
  }, [tabs, activeId]);

  const active = tabs.find(entry => entry.id === activeId) ?? tabs[0];
  const overflow = useRibbonOverflow(panelsRef, `${active?.id ?? ''}/${locale}`);
  const reserve = ribbonCueReserve(overflow);

  // Dismiss an open variant popover on Escape or a click outside it, exactly
  // like the drafting band.
  useEffect(() => {
    if (!openMenu) return undefined;
    const onKeyDown = (event: KeyboardEvent) => { if (event.key === 'Escape') setOpenMenu(null); };
    const onPointerDown = (event: PointerEvent) => {
      const target = event.target as Element | null;
      if (!target?.closest?.('.ribbon-panel__menu')) setOpenMenu(null);
    };
    document.addEventListener('keydown', onKeyDown);
    document.addEventListener('pointerdown', onPointerDown);
    return () => {
      document.removeEventListener('keydown', onKeyDown);
      document.removeEventListener('pointerdown', onPointerDown);
    };
  }, [openMenu]);

  if (!active) return null;
  const panels = offeredPanels(active.panels, shell.palettesAvailable);
  return (
    <div className="region region--ribbon ribbon domain-ribbon app-region">
      <div className="ribbon__tabs" role="tablist" aria-label={t('ribbon.tabs_label')}>
        {tabs.map(entry => (
          <button
            key={entry.id}
            type="button"
            role="tab"
            aria-selected={entry.id === active.id}
            className={['ribbon-tab', entry.accent ? 'is-accent' : '', entry.id === active.id ? 'is-active' : ''].filter(Boolean).join(' ')}
            onClick={() => { setOpenMenu(null); setActiveId(entry.id); }}
          >
            {tabLabel(t, entry)}
          </button>
        ))}
        <div className="ribbon__tabs-spacer" />
      </div>
      <div
        className="ribbon__band"
        style={{ '--ribbon-cue-start': `${reserve.start}px`, '--ribbon-cue-end': `${reserve.end}px` } as React.CSSProperties}
      >
        <div className="ribbon__panels" role="tabpanel" aria-label={tabLabel(t, active)} ref={panelsRef}>
          {panels.map(panel => (
            <RibbonPanelView key={panel.id} panel={panel} openMenu={openMenu} setOpenMenu={setOpenMenu} />
          ))}
        </div>
        {overflow.start ? <OverflowCue side="start" onPage={() => pageRibbon(panelsRef.current, 'start')} /> : null}
        {overflow.end ? <OverflowCue side="end" onPage={() => pageRibbon(panelsRef.current, 'end')} /> : null}
      </div>
    </div>
  );
}
