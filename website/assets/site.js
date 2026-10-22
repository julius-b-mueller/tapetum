// German/English: elements with data-de/data-en get their text, blocks with
// class de/en are shown or hidden. The choice is remembered; the first visit
// follows the browser language.
(function () {
  var stored = null;
  try { stored = localStorage.getItem('tapetum-lang') } catch (e) {}
  var lang = stored || ((navigator.language || 'de').toLowerCase().indexOf('de') === 0 ? 'de' : 'en');
  window.setLang = function (l) {
    lang = l;
    try { localStorage.setItem('tapetum-lang', l) } catch (e) {}
    document.documentElement.lang = l;
    document.querySelectorAll('.lang-btn').forEach(function (b) { b.classList.toggle('active', b.dataset.lang === l) });
    document.querySelectorAll('.de').forEach(function (e) { e.style.display = l === 'de' ? '' : 'none' });
    document.querySelectorAll('.en').forEach(function (e) { e.style.display = l === 'en' ? '' : 'none' });
    document.querySelectorAll('[data-de]').forEach(function (e) { var t = e.getAttribute('data-' + l); if (t) e.innerHTML = t });
    document.querySelectorAll('[data-badge-de]').forEach(function (e) { e.setAttribute('data-badge', e.getAttribute('data-badge-' + l)) });
  };
  // Tabs of the "unsigned" note at the downloads.
  window.showWarn = function (id) {
    document.querySelectorAll('.warn-tab').forEach(function (t) { t.classList.toggle('active', t.dataset.tab === id) });
    document.querySelectorAll('.warn-body > div').forEach(function (d) { d.classList.toggle('hidden', d.id !== 'warn-' + id) });
  };
  document.addEventListener('DOMContentLoaded', function () {
    // Point the downloads and the "unsigned" note at the visitor's system.
    var ua = navigator.userAgent, p = /Mac/.test(ua) ? 'mac' : /Win/.test(ua) ? 'win' : /Linux/.test(ua) ? 'lin' : '';
    var card = p && document.querySelector('.dl [data-platform="' + p + '"]');
    if (card) {
      card.classList.add('recommended');
      card.querySelector('h3').setAttribute('data-badge-de', 'Dein System');
      card.querySelector('h3').setAttribute('data-badge-en', 'Your system');
    }
    if (p && document.getElementById('warn-' + p)) showWarn(p);
    setLang(lang);
  });
})();
