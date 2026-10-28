document.querySelectorAll('.details-toggle').forEach((button) => {
  button.addEventListener('click', () => {
    const wasExpanded = button.getAttribute('aria-expanded') === 'true';
    document.querySelectorAll('.details-toggle[aria-expanded="true"]').forEach((openButton) => {
      openButton.setAttribute('aria-expanded', 'false');
      openButton.closest('.site-row').nextElementSibling.hidden = true;
    });
    if (!wasExpanded) {
      button.setAttribute('aria-expanded', 'true');
      button.closest('.site-row').nextElementSibling.hidden = false;
    }
  });
});

document.querySelectorAll('.copy-link').forEach((button) => {
  button.addEventListener('click', async () => {
    const actions = button.closest('.details-actions');
    const feedback = actions.nextElementSibling;
    try {
      await navigator.clipboard.writeText(button.dataset.copyUrl);
      feedback.textContent = 'Link copied';
    } catch (_) {
      feedback.textContent = 'Could not copy link';
    }
  });
});
