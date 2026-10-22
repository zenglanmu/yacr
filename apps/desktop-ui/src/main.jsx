// Entry point: mount the desktop shell. Behaviour lives in `App.jsx`, visual
// regions under `components/`.
import React from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './App.jsx';
import './style.css';
import { DomOnTopProbe } from './probe-dom-on-top/Probe.jsx';
import { isDomOnTopProbe } from './probe-dom-on-top/state.mjs';

createRoot(document.getElementById('root')).render(
  isDomOnTopProbe(window) ? <DomOnTopProbe /> : <App />,
);
