// Operational View Route
// Phase 4.3: User Experience Enhancement

import OperationalView from '../components/operational/OperationalView';

export const operationalRoute = {
  path: '/operational',
  component: OperationalView,
  meta: {
    title: 'Operational View',
    description: 'Daily priority queue, supplier risk monitor, pipeline tracker, and alert management',
    roles: ['operator', 'analyst', 'admin'],
  },
};

export const operationalRoutes = [
  {
    path: '/operational',
    name: 'Daily Operations',
    component: OperationalView,
    exact: true,
  },
  {
    path: '/operational/queue',
    name: 'Priority Queue',
    component: OperationalView,
  },
  {
    path: '/operational/suppliers',
    name: 'Supplier Risks',
    component: OperationalView,
  },
  {
    path: '/operational/pipeline',
    name: 'Pipeline',
    component: OperationalView,
  },
  {
    path: '/operational/alerts',
    name: 'Alert Management',
    component: OperationalView,
  },
];

export default OperationalView;
