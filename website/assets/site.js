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
  };
  document.addEventListener('DOMContentLoaded', function () {
    setLang(lang);
    // Point the big download button at the visitor's system.
    var ua = navigator.userAgent, p = /Mac/.test(ua) ? 'mac' : /Win/.test(ua) ? 'win' : /Linux/.test(ua) ? 'lin' : '';
    var card = p && document.querySelector('.dl [data-platform="' + p + '"]');
    if (card) card.classList.add('recommended');
  });
})();
