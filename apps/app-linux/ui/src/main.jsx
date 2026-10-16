// Entry point: mount the desktop shell. Behaviour lives in `App.jsx`, visual
// regions under `components/`.
import React from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './App.jsx';
import './style.css';

createRoot(document.getElementById('root')).render(<App />);
