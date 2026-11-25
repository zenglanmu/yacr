// The top menu bar slot.
//
// It is the same `RibbonMenuBar` the collapsed drafting ribbon uses, handed the
// current domain's menu model. That keeps one menu renderer (keyboard model,
// placement, refusal copy and dispatch) instead of a second implementation for
// SketchUp-style menus. A profile with no `menu` slot draws nothing.
import React from 'react';
import { useShell } from '../../context.ts';
import { RibbonMenuBar } from '../../components/RibbonMenuBar.tsx';
import { currentDomain } from '../store.ts';
import { useDomainUi } from './use-ui-layout.ts';

export function DomainMenuBar() {
  const shell = useShell();
  const domain = currentDomain(shell.status);
  const layout = useDomainUi(domain);
  if (!layout.menu || layout.menu.length === 0) return null;
  return (
    <div className="domain-menubar" data-domain-menu={domain}>
      <RibbonMenuBar tabs={layout.menu} showExpand={false} labelKey="domain.menu_label" />
    </div>
  );
}
