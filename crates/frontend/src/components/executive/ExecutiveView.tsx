import { useState, useEffect } from 'react';

// API types for executive dashboard
interface StrategicOpportunity {
  id: string;
  title: string;
  description?: string;
  opportunity_type: string;
  priority_score: number;
  confidence: number;
  entity_id?: string;
  entity_type?: string;
  region?: string;
  estimated_value?: string;
  recommended_actions: string[];
  owner_id?: string;
  status: string;
  due_date?: string;
}

interface CriticalThreat {
  id: string;
  title: string;
  description?: string;
  threat_type: string;
  severity: string;
  impact_score: number;
  confidence: number;
  entity_id?: string;
  entity_type?: string;
  region?: string;
  mitigation_steps: string[];
  owner_id?: string;
  status: string;
  sla_deadline?: string;
}

interface MarketIntelligenceSummary {
  total_opportunities: number;
  total_threats: number;
  high_priority_count: number;
  regions_affected: string[];
  average_confidence: number;
}

interface RecommendedAction {
  id: string;
  title: string;
  description: string;
  priority: string;
  owner: string;
  due_date?: string;
  related_entity_id?: string;
  related_entity_type?: string;
}

interface ExecutiveSummary {
  top_opportunities: StrategicOpportunity[];
  critical_threats: CriticalThreat[];
  market_intelligence: MarketIntelligenceSummary;
  recommended_actions: RecommendedAction[];
}

interface ApiEnvelope<T> {
  success: boolean;
  data?: T;
  error?: { code: string; message: string };
}

// Severity badge component
const SeverityBadge: React.FC<{ severity: string }> = ({ severity }) => {
  const styles: Record<string, string> = {
    low: 'bg-green-100 text-green-800 border-green-200',
    medium: 'bg-yellow-100 text-yellow-800 border-yellow-200',
    high: 'bg-orange-100 text-orange-800 border-orange-200',
    critical: 'bg-red-100 text-red-800 border-red-200',
  };
  
  return (
    <span className={`px-2 py-1 rounded-full text-xs font-medium border ${styles[severity] || styles.medium}`}>
      {severity.toUpperCase()}
    </span>
  );
};

// Priority score display
const PriorityScore: React.FC<{ score: number; type: 'opportunity' | 'threat' }> = ({ score, type }) => {
  const colorClass = score > 0.8 ? 'text-red-600' : score > 0.5 ? 'text-yellow-600' : 'text-green-600';
  
  return (
    <div className="flex items-center gap-2">
      <div className="w-16 h-2 bg-gray-200 rounded-full overflow-hidden">
        <div 
          className={`h-full rounded-full ${score > 0.8 ? 'bg-red-500' : score > 0.5 ? 'bg-yellow-500' : 'bg-green-500'}`}
          style={{ width: `${score * 100}%` }}
        />
      </div>
      <span className={`text-sm font-semibold ${colorClass}`}>
        {Math.round(score * 100)}%
      </span>
    </div>
  );
};

// Market intelligence card
const MarketIntelCard: React.FC<{ summary: MarketIntelligenceSummary }> = ({ summary }) => {
  return (
    <div className="bg-white rounded-lg shadow p-6">
      <h3 className="text-lg font-semibold text-gray-900 mb-4">Market Intelligence Summary</h3>
      <div className="grid grid-cols-2 gap-4">
        <div className="p-4 bg-blue-50 rounded-lg">
          <p className="text-sm text-gray-600">Total Opportunities</p>
          <p className="text-2xl font-bold text-blue-600">{summary.total_opportunities}</p>
        </div>
        <div className="p-4 bg-red-50 rounded-lg">
          <p className="text-sm text-gray-600">Critical Threats</p>
          <p className="text-2xl font-bold text-red-600">{summary.total_threats}</p>
        </div>
        <div className="p-4 bg-purple-50 rounded-lg">
          <p className="text-sm text-gray-600">High Priority Items</p>
          <p className="text-2xl font-bold text-purple-600">{summary.high_priority_count}</p>
        </div>
        <div className="p-4 bg-gray-50 rounded-lg">
          <p className="text-sm text-gray-600">Avg Confidence</p>
          <p className="text-2xl font-bold text-gray-600">{Math.round(summary.average_confidence * 100)}%</p>
        </div>
      </div>
      {summary.regions_affected.length > 0 && (
        <div className="mt-4">
          <p className="text-sm text-gray-600 mb-2">Regions Affected:</p>
          <div className="flex flex-wrap gap-2">
            {summary.regions_affected.map((region) => (
              <span key={region} className="px-3 py-1 bg-gray-100 text-gray-700 rounded-full text-sm">
                {region}
              </span>
            ))}
          </div>
        </div>
      )}
    </div>
  );
};

// Strategic opportunity card
const OpportunityCard: React.FC<{ opportunity: StrategicOpportunity }> = ({ opportunity }) => {
  return (
    <div className="bg-white rounded-lg shadow hover:shadow-md transition-shadow p-4 border-l-4 border-blue-500">
      <div className="flex justify-between items-start mb-2">
        <h4 className="text-lg font-semibold text-gray-900">{opportunity.title}</h4>
        <span className={`px-2 py-1 rounded text-xs font-medium ${
          opportunity.priority_score > 0.8 ? 'bg-red-100 text-red-700' :
          opportunity.priority_score > 0.5 ? 'bg-yellow-100 text-yellow-700' :
          'bg-green-100 text-green-700'
        }`}>
          {Math.round(opportunity.priority_score * 100)}%
        </span>
      </div>
      {opportunity.description && (
        <p className="text-sm text-gray-600 mb-3">{opportunity.description}</p>
      )}
      <div className="flex flex-wrap gap-2 mb-3">
        <span className="px-2 py-1 bg-blue-100 text-blue-700 rounded text-xs">
          {opportunity.opportunity_type.replace(/_/g, ' ')}
        </span>
        {opportunity.region && (
          <span className="px-2 py-1 bg-gray-100 text-gray-700 rounded text-xs">
            {opportunity.region}
          </span>
        )}
      </div>
      <PriorityScore score={opportunity.priority_score} type="opportunity" />
      {opportunity.recommended_actions.length > 0 && (
        <div className="mt-3 pt-3 border-t border-gray-100">
          <p className="text-xs text-gray-500 mb-2">Recommended Actions:</p>
          <ul className="text-xs text-gray-700 space-y-1">
            {opportunity.recommended_actions.slice(0, 3).map((action, idx) => (
              <li key={idx} className="flex items-start gap-2">
                <span className="text-blue-500">→</span>
                {action}
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
};

// Critical threat card
const ThreatCard: React.FC<{ threat: CriticalThreat }> = ({ threat }) => {
  return (
    <div className="bg-white rounded-lg shadow hover:shadow-md transition-shadow p-4 border-l-4 border-red-500">
      <div className="flex justify-between items-start mb-2">
        <h4 className="text-lg font-semibold text-gray-900">{threat.title}</h4>
        <SeverityBadge severity={threat.severity} />
      </div>
      {threat.description && (
        <p className="text-sm text-gray-600 mb-3">{threat.description}</p>
      )}
      <div className="flex flex-wrap gap-2 mb-3">
        <span className="px-2 py-1 bg-red-100 text-red-700 rounded text-xs">
          {threat.threat_type.replace(/_/g, ' ')}
        </span>
        {threat.region && (
          <span className="px-2 py-1 bg-gray-100 text-gray-700 rounded text-xs">
            {threat.region}
          </span>
        )}
      </div>
      <PriorityScore score={threat.impact_score} type="threat" />
      {threat.mitigation_steps.length > 0 && (
        <div className="mt-3 pt-3 border-t border-gray-100">
          <p className="text-xs text-gray-500 mb-2">Mitigation Steps:</p>
          <ul className="text-xs text-gray-700 space-y-1">
            {threat.mitigation_steps.slice(0, 3).map((step, idx) => (
              <li key={idx} className="flex items-start gap-2">
                <span className="text-red-500">⚠</span>
                {step}
              </li>
            ))}
          </ul>
        </div>
      )}
      {threat.sla_deadline && (
        <div className="mt-3 pt-3 border-t border-gray-100">
          <p className="text-xs text-gray-500">
            SLA Deadline: <span className="font-medium text-red-600">
              {new Date(threat.sla_deadline).toLocaleDateString()}
            </span>
          </p>
        </div>
      )}
    </div>
  );
};

// Recommended action card
const ActionCard: React.FC<{ action: RecommendedAction }> = ({ action }) => {
  const priorityColors: Record<string, string> = {
    critical: 'bg-red-100 text-red-700 border-red-200',
    high: 'bg-orange-100 text-orange-700 border-orange-200',
    medium: 'bg-yellow-100 text-yellow-700 border-yellow-200',
    low: 'bg-green-100 text-green-700 border-green-200',
  };
  
  return (
    <div className="bg-white rounded-lg shadow p-4 flex items-center gap-4">
      <div className={`px-3 py-1 rounded text-xs font-medium border ${priorityColors[action.priority] || priorityColors.medium}`}>
        {action.priority.toUpperCase()}
      </div>
      <div className="flex-1">
        <h4 className="font-semibold text-gray-900">{action.title}</h4>
        {action.description && (
          <p className="text-sm text-gray-600">{action.description}</p>
        )}
      </div>
      <div className="text-right">
        <p className="text-xs text-gray-500">Owner: {action.owner}</p>
        {action.due_date && (
          <p className="text-xs text-gray-500">
            Due: {new Date(action.due_date).toLocaleDateString()}
          </p>
        )}
      </div>
    </div>
  );
};

// Main executive view component
export const ExecutiveView: React.FC = () => {
  const [data, setData] = useState<ExecutiveSummary | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState<'opportunities' | 'threats' | 'actions'>('opportunities');

  useEffect(() => {
    fetchExecutiveSummary();
  }, []);

  const fetchExecutiveSummary = async () => {
    try {
      setLoading(true);
      const response = await fetch('/api/v1/collaboration/executive-summary', {
        headers: {
          'Authorization': `Bearer ${localStorage.getItem('api_token')}`,
        },
      });

      if (!response.ok) {
        throw new Error('Failed to fetch executive summary');
      }

      const envelope: ApiEnvelope<ExecutiveSummary> = await response.json();
      
      if (envelope.success && envelope.data) {
        setData(envelope.data);
      } else {
        throw new Error(envelope.error?.message || 'Unknown error');
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load dashboard');
    } finally {
      setLoading(false);
    }
  };

  if (loading) {
    return (
      <div className="flex items-center justify-center h-screen">
        <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-blue-600"></div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="p-6">
        <div className="bg-red-50 border border-red-200 rounded-lg p-4 text-red-700">
          <p className="font-semibold">Error loading dashboard</p>
          <p className="text-sm">{error}</p>
          <button 
            onClick={() => fetchExecutiveSummary()}
            className="mt-2 px-4 py-2 bg-red-600 text-white rounded hover:bg-red-700"
          >
            Retry
          </button>
        </div>
      </div>
    );
  }

  if (!data) {
    return null;
  }

  return (
    <div className="min-h-screen bg-gray-50">
      <header className="bg-white shadow-sm">
        <div className="max-w-7xl mx-auto px-4 py-4 flex justify-between items-center">
          <div>
            <h1 className="text-2xl font-bold text-gray-900">Executive Dashboard</h1>
            <p className="text-sm text-gray-500">Strategic overview and priority actions</p>
          </div>
          <div className="flex gap-2">
            <button 
              onClick={() => fetchExecutiveSummary()}
              className="px-4 py-2 bg-gray-100 text-gray-700 rounded hover:bg-gray-200"
            >
              Refresh
            </button>
            <button className="px-4 py-2 bg-blue-600 text-white rounded hover:bg-blue-700">
              Export Report
            </button>
          </div>
        </div>
      </header>

      <main className="max-w-7xl mx-auto px-4 py-6">
        <section className="mb-8">
          <MarketIntelCard summary={data.market_intelligence} />
        </section>

        <div className="flex gap-4 mb-6 border-b border-gray-200">
          <button
            onClick={() => setActiveTab('opportunities')}
            className={`px-4 py-2 font-medium border-b-2 ${
              activeTab === 'opportunities'
                ? 'border-blue-600 text-blue-600'
                : 'border-transparent text-gray-500 hover:text-gray-700'
            }`}
          >
            Opportunities ({data.top_opportunities.length})
          </button>
          <button
            onClick={() => setActiveTab('threats')}
            className={`px-4 py-2 font-medium border-b-2 ${
              activeTab === 'threats'
                ? 'border-red-600 text-red-600'
                : 'border-transparent text-gray-500 hover:text-gray-700'
            }`}
          >
            Threats ({data.critical_threats.length})
          </button>
          <button
            onClick={() => setActiveTab('actions')}
            className={`px-4 py-2 font-medium border-b-2 ${
              activeTab === 'actions'
                ? 'border-purple-600 text-purple-600'
                : 'border-transparent text-gray-500 hover:text-gray-700'
            }`}
          >
            Recommended Actions ({data.recommended_actions.length})
          </button>
        </div>

        <section>
          {activeTab === 'opportunities' && (
            <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
              {data.top_opportunities.map((opp) => (
                <OpportunityCard key={opp.id} opportunity={opp} />
              ))}
              {data.top_opportunities.length === 0 && (
                <p className="text-gray-500 text-center py-8">No opportunities found</p>
              )}
            </div>
          )}

          {activeTab === 'threats' && (
            <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
              {data.critical_threats.map((threat) => (
                <ThreatCard key={threat.id} threat={threat} />
              ))}
              {data.critical_threats.length === 0 && (
                <p className="text-gray-500 text-center py-8">No threats found</p>
              )}
            </div>
          )}

          {activeTab === 'actions' && (
            <div className="space-y-3">
              {data.recommended_actions
                .sort((a, b) => {
                  const priorityOrder = { critical: 0, high: 1, medium: 2, low: 3 };
                  return (priorityOrder[a.priority] || 4) - (priorityOrder[b.priority] || 4);
                })
                .map((action) => (
                  <ActionCard key={action.id} action={action} />
                ))}
              {data.recommended_actions.length === 0 && (
                <p className="text-gray-500 text-center py-8">No actions recommended</p>
              )}
            </div>
          )}
        </section>
      </main>
    </div>
  );
};

export default ExecutiveView;
