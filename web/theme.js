try {
  const theme = localStorage.getItem('recorder-theme');
  if (['light', 'dark', 'system'].includes(theme)) document.documentElement.dataset.theme = theme;
} catch { /* Server settings are still available when browser storage is disabled. */ }
