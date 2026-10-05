// Canonical server-UI route + viewport inventory.
//
// Every UI dogfood harness (route sweep, accessibility gate, adversarial scan)
// imports this file, so adding a route to the app forces one edit here and all
// scans cover it. Keep this list aligned with the router's navigation surface.
'use strict';

const ROUTES = [
  '/',
  '/warnings',
  '/insights',
  '/companies',
  '/persons',
  '/buying-centers',
  '/competitors',
  '/battlecards',
  '/search',
  '/graph',
  '/triage',
  '/workspaces',
  '/queue',
  '/activity',
  '/supplier-risk',
  '/pipeline',
  '/evidence',
  '/team-assignments',
  '/executive',
  '/trends',
  '/security',
  '/admin',
  '/memos',
  '/notifications',
  '/settings',
  '/settings/alerts',
  '/recipes',
  '/recipes/new',
];

const VIEWPORTS = [
  { name: 'mobile', width: 390, height: 844 },
  { name: 'tablet', width: 834, height: 1112 },
  { name: 'desktop', width: 1440, height: 900 },
];

module.exports = { ROUTES, VIEWPORTS };
