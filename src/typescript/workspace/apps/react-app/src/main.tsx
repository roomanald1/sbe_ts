import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import '@workspace/ui-platform/styles/theme.css';
import '@workspace/ui-platform/styles/dashboard_widgets.css';
import './index.css';
import App from './App.tsx';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

const queryClient = new QueryClient();

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <App />
    </QueryClientProvider>
  </StrictMode>,
);
