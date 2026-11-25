// Service worker mínimo: torna o Capivara instalável como app (PWA) no celular/desktop.
// Tudo passa direto pela rede — o app depende do servidor para baixar.
self.addEventListener('install', () => self.skipWaiting());
self.addEventListener('activate', (e) => e.waitUntil(self.clients.claim()));
self.addEventListener('fetch', () => {});
