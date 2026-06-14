// Executive View Dashboard Route
// Phase 4.3: User Experience Enhancement

import ExecutiveView from '../components/executive/ExecutiveView';

export const executiveRoute = {
  path: '/executive',
  component: ExecutiveView,
  meta: {
    title: 'Executive Dashboard',
    description: 'Strategic overview with top opportunities, critical threats, and recommended actions',
    roles: ['executive', 'admin', 'analyst'],
  },
};

export const executiveRoutes = [
  {
    path: '/executive',
    name: 'Executive Dashboard',
    component: ExecutiveView,
    exact: true,
  },
  {
    path: '/executive/opportunities',
    name: 'Strategic Opportunities',
    component: ExecutiveView,
  },
  {
    path: '/executive/threats',
    name: 'Critical Threats',
    component: ExecutiveView,
  },
  {
    path: '/executive/actions',
    name: 'Recommended Actions',
    component: ExecutiveView,
  },
];

export default ExecutiveView;
