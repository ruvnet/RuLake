import React from 'react';
import * as ReactDOM from 'react-dom/client';
import { RuLakeHttp, RuLakeHttpError } from './lib/http.js';
import './styles/styles.css';
import './styles/help.css';

Object.assign(window, { React, ReactDOM, RuLakeHttp, RuLakeHttpError });

// The design modules read their globals during evaluation. Initialize those
// first, then load the console as a unit. WASM and workers remain on demand.
import('./console.jsx').catch(error => {
  console.error('[rulake-console] bootstrap failed:', error);
  const root = document.getElementById('root');
  if (root) root.textContent = 'Console failed to start: ' + (error.message || String(error));
});
