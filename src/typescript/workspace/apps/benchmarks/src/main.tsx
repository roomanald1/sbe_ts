import React from 'react';
import { createRoot } from 'react-dom/client';
import App from './views/App';
import '@workspace/ui-platform/styles/theme.css';
import './styles.css';

createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
