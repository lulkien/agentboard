(() => {
  const button = document.querySelector('[data-theme-toggle]');
  if (!button) return;
  const allowed = ['light', 'dark', 'system'];
  let theme = 'system';
  try {
    const saved = localStorage.getItem('agentboard-theme');
    if (allowed.includes(saved)) theme = saved;
  } catch (_) {}
  const apply = () => {
    if (theme === 'system') document.documentElement.removeAttribute('data-theme');
    else document.documentElement.setAttribute('data-theme', theme);
    button.textContent = `Theme: ${theme}`;
    button.setAttribute('aria-pressed', String(theme !== 'system'));
    button.setAttribute('aria-label', `Color theme: ${theme}; activate to change`);
  };
  apply();
  button.addEventListener('click', () => {
    theme = allowed[(allowed.indexOf(theme) + 1) % allowed.length];
    apply();
    try { if (theme === 'system') localStorage.removeItem('agentboard-theme'); else localStorage.setItem('agentboard-theme', theme); } catch (_) {}
  });
})();
