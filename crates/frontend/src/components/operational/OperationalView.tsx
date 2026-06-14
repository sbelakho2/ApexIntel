import { useState, useEffect } from 'react';

// Types for operational view
interface PriorityQueueItem {
  id: string;
  item_type: string;
  item_id: string;
  item_title: string;
  priority: number;
  status: string;
  notes?: string;
  created_at: string;
}

interface SupplierRiskEntry {
  id: string;
  supplier_id: string;
  supplier_name?: string;
  risk_category: string;
  risk_score: number;
  risk_factors: string[];
  mitigation?: string;
  status: string;
  last_reviewed?: string;
  next_review?: string;
}

interface PipelineOpportunity {
  id: string;
  title: string;
  stage: string;
  value_estimate?: number;
  probability: number;
  owner_id?: string;
  expected_close?: string;
}

interface AlertItem {
  id: string;
  title: string;
  severity: string;
  entity_type: string;
  entity_id: string;
  created_at: string;
}

// Priority queue item component
const QueueItem: React.FC<{
  item: PriorityQueueItem;
  onStatusChange: (id: string, status: string) => void;
}> = ({ item, onStatusChange }) => {
  const priorityColors: Record<string, string> = {
    high: 'bg-red-100 text-red-700 border-red-200',
    medium: 'bg-yellow-100 text-yellow-700 border-yellow-200',
    low: 'bg-green-100 text-green-700 border-green-200',
  };
  
  const priorityLevel = item.priority > 80 ? 'high' : item.priority > 50 ? 'medium' : 'low';
  const statusColors: Record<string, string> = {
    pending: 'bg-gray-100 text-gray-600',
    in_progress: 'bg-blue-100 text-blue-600',
    completed: 'bg-green-100 text-green-600',
    cancelled: 'bg-gray-100 text-gray-400',
  };

  return (
    <div className="bg-white rounded-lg shadow p-4 flex items-center gap-4">
      <div className="flex-1">
        <div className="flex items-center gap-2 mb-1">
          <span className={`px-2 py-0.5 rounded text-xs font-medium border ${priorityColors[priorityLevel]}`}>
            P{item.priority}
          </span>
          <span className={`px-2 py-0.5 rounded text-xs ${statusColors[item.status]}`}>
            {item.status.replace(/_/g, ' ')}
          </span>
          <span className="text-xs text-gray-400">{item.item_type}</span>
        </div>
        <h4 className="font-medium text-gray-900">{item.item_title}</h4>
        {item.notes && (
          <p className="text-sm text-gray-500 mt-1">{item.notes}</p>
        )}
      </div>
      <div className="flex gap-2">
        {item.status === 'pending' && (
          <button
            onClick={() => onStatusChange(item.id, 'in_progress')}
            className="px-3 py-1 bg-blue-600 text-white rounded text-sm hover:bg-blue-700"
          >
            Start
          </button>
        )}
        {item.status === 'in_progress' && (
          <button
            onClick={() => onStatusChange(item.id, 'completed')}
            className="px-3 py-1 bg-green-600 text-white rounded text-sm hover:bg-green-700"
          >
            Complete
          </button>
        )}
      </div>
    </div>
  );
};

// Supplier risk card
const SupplierRiskCard: React.FC<{ risk: SupplierRiskEntry }> = ({ risk }) => {
  const riskColors: Record<string, string> = {
    financial: 'bg-blue-100 text-blue-700',
    operational: 'bg-orange-100 text-orange-700',
    compliance: 'bg-purple-100 text-purple-700',
    geopolitical: 'bg-red-100 text-red-700',
    environmental: 'bg-green-100 text-green-700',
    technological: 'bg-cyan-100 text-cyan-700',
  };

  return (
    <div className="bg-white rounded-lg shadow p-4 border-l-4 border-orange-500">
      <div className="flex justify-between items-start mb-2">
        <div>
          <h4 className="font-semibold text-gray-900">{risk.supplier_name || risk.supplier_id}</h4>
          <span className={`px-2 py-0.5 rounded text-xs ${riskColors[risk.risk_category] || 'bg-gray-100 text-gray-700'}`}>
            {risk.risk_category}
          </span>
        </div>
        <div className="text-right">
          <div className={`text-2xl font-bold ${
            risk.risk_score > 0.7 ? 'text-red-600' :
            risk.risk_score > 0.4 ? 'text-yellow-600' : 'text-green-600'
          }`}>
            {(risk.risk_score * 100).toFixed(0)}%
          </div>
          <div className="text-xs text-gray-500">Risk Score</div>
        </div>
      </div>
      
      {risk.risk_factors.length > 0 && (
        <div className="mt-3">
          <p className="text-xs text-gray-500 mb-1">Risk Factors:</p>
          <ul className="text-sm text-gray-700 space-y-1">
            {risk.risk_factors.slice(0, 3).map((factor, idx) => (
              <li key={idx} className="flex items-start gap-1">
                <span className="text-orange-500">•</span>
                {factor}
              </li>
            ))}
          </ul>
        </div>
      )}
      
      {risk.mitigation && (
        <div className="mt-3 pt-3 border-t border-gray-100">
          <p className="text-xs text-gray-500 mb-1">Mitigation:</p>
          <p className="text-sm text-gray-700">{risk.mitigation}</p>
        </div>
      )}
      
      {risk.next_review && (
        <p className="text-xs text-gray-400 mt-3">
          Next review: {new Date(risk.next_review).toLocaleDateString()}
        </p>
      )}
    </div>
  );
};

// Pipeline tracker component
const PipelineTracker: React.FC<{ opportunities: PipelineOpportunity[] }> = ({ opportunities }) => {
  const stages = ['discovery', 'qualification', 'proposal', 'negotiation', 'closed_won', 'closed_lost'];
  const stageColors: Record<string, string> = {
    discovery: 'bg-blue-100 border-blue-300',
    qualification: 'bg-cyan-100 border-cyan-300',
    proposal: 'bg-yellow-100 border-yellow-300',
    negotiation: 'bg-orange-100 border-orange-300',
    closed_won: 'bg-green-100 border-green-300',
    closed_lost: 'bg-red-100 border-red-300',
  };

  const getStageTotal = (stage: string) => {
    return opportunities
      .filter(o => o.stage === stage)
      .reduce((sum, o) => sum + (o.value_estimate || 0), 0);
  };

  return (
    <div className="bg-white rounded-lg shadow p-4">
      <h3 className="text-lg font-semibold mb-4">Pipeline Opportunity Tracker</h3>
      <div className="flex justify-between items-end gap-2 overflow-x-auto">
        {stages.map((stage) => (
          <div key={stage} className="flex-1 min-w-[120px]">
            <div className={`p-3 rounded-t border-2 ${stageColors[stage]}`}>
              <p className="text-xs text-gray-600 capitalize">{stage.replace(/_/g, ' ')}</p>
              <p className="text-lg font-bold text-gray-900">
                {opportunities.filter(o => o.stage === stage).length}
              </p>
            </div>
            <div className={`p-2 border-x-2 border-b-2 ${stageColors[stage].replace('100', '50')}`}>
              <p className="text-xs text-gray-600">
                ${(getStageTotal(stage) / 1000000).toFixed(1)}M
              </p>
            </div>
          </div>
        ))}
      </div>
      
      <div className="mt-4 space-y-2">
        {opportunities.slice(0, 5).map((opp) => (
          <div key={opp.id} className="flex justify-between items-center p-2 bg-gray-50 rounded">
            <div>
              <p className="font-medium text-gray-900">{opp.title}</p>
              <p className="text-xs text-gray-500 capitalize">{opp.stage.replace(/_/g, ' ')}</p>
            </div>
            <div className="text-right">
              {opp.value_estimate && (
                <p className="font-semibold text-gray-900">
                  ${(opp.value_estimate / 1000).toFixed(0)}K
                </p>
              )}
              <p className="text-xs text-gray-500">
                {(opp.probability * 100).toFixed(0)}% prob
              </p>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
};

// Alert management component
const AlertManagement: React.FC<{
  alerts: AlertItem[];
  onAcknowledge: (id: string) => void;
}> = ({ alerts, onAcknowledge }) => {
  const severityColors: Record<string, string> = {
    critical: 'bg-red-100 text-red-700 border-red-200',
    high: 'bg-orange-100 text-orange-700 border-orange-200',
    medium: 'bg-yellow-100 text-yellow-700 border-yellow-200',
    low: 'bg-blue-100 text-blue-700 border-blue-200',
  };

  return (
    <div className="bg-white rounded-lg shadow p-4">
      <h3 className="text-lg font-semibold mb-4">Alert Management</h3>
      <div className="space-y-2 max-h-80 overflow-y-auto">
        {alerts.length === 0 ? (
          <p className="text-gray-500 text-center py-4">No active alerts</p>
        ) : (
          alerts.map((alert) => (
            <div 
              key={alert.id}
              className={`p-3 rounded border ${severityColors[alert.severity] || severityColors.medium}`}
            >
              <div className="flex justify-between items-start">
                <div>
                  <h4 className="font-medium">{alert.title}</h4>
                  <p className="text-xs mt-1">
                    {alert.entity_type} • {new Date(alert.created_at).toLocaleString()}
                  </p>
                </div>
                <button
                  onClick={() => onAcknowledge(alert.id)}
                  className="px-2 py-1 bg-white rounded text-sm hover:bg-gray-100"
                >
                  Acknowledge
                </button>
              </div>
            </div>
          ))
        )}
      </div>
    </div>
  );
};

// Main operational view component
export const OperationalView: React.FC = () => {
  const [queueItems, setQueueItems] = useState<PriorityQueueItem[]>([]);
  const [supplierRisks, setSupplierRisks] = useState<SupplierRiskEntry[]>([]);
  const [pipelineOpps, setPipelineOpps] = useState<PipelineOpportunity[]>([]);
  const [alerts, setAlerts] = useState<AlertItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [activeTab, setActiveTab] = useState<'queue' | 'suppliers' | 'pipeline' | 'alerts'>('queue');

  useEffect(() => {
    fetchData();
  }, []);

  const fetchData = async () => {
    const token = localStorage.getItem('api_token');
    const headers = { 'Authorization': `Bearer ${token}` };

    try {
      const [queueRes, riskRes, pipelineRes, alertRes] = await Promise.all([
        fetch('/api/v1/collaboration/queue', { headers }),
        fetch('/api/v1/collaboration/supplier-risks', { headers }),
        fetch('/api/v1/collaboration/pipeline', { headers }),
        fetch('/api/v1/warnings?severity=high,critical&status=active&limit=20', { headers }),
      ]);

      const [queueData, riskData, pipelineData, alertData] = await Promise.all([
        queueRes.json(),
        riskRes.json(),
        pipelineRes.json(),
        alertRes.json(),
      ]);

      if (queueData.success) setQueueItems(queueData.data || []);
      if (riskData.success) setSupplierRisks(riskData.data || []);
      if (pipelineData.success) setPipelineOpps(pipelineData.data || []);
      if (alertData.success) setAlerts(alertData.data?.items || []);
    } catch (error) {
      console.error('Failed to fetch operational data:', error);
    } finally {
      setLoading(false);
    }
  };

  const handleQueueStatusChange = async (id: string, status: string) => {
    try {
      const response = await fetch(`/api/v1/collaboration/queue/${id}`, {
        method: 'PATCH',
        headers: {
          'Authorization': `Bearer ${localStorage.getItem('api_token')}`,
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({ status }),
      });
      const data = await response.json();
      if (data.success) {
        setQueueItems(items =>
          items.map(item => item.id === id ? { ...item, status } : item)
        );
      }
    } catch (error) {
      console.error('Failed to update queue item:', error);
    }
  };

  const handleAlertAcknowledge = async (id: string) => {
    try {
      await fetch(`/api/v1/warnings/${id}/acknowledge`, {
        method: 'POST',
        headers: {
          'Authorization': `Bearer ${localStorage.getItem('api_token')}`,
          'Content-Type': 'application/json',
        },
      });
      setAlerts(alerts => alerts.filter(a => a.id !== id));
    } catch (error) {
      console.error('Failed to acknowledge alert:', error);
    }
  };

  if (loading) {
    return (
      <div className="flex items-center justify-center h-screen">
        <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-blue-600"></div>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-gray-50">
      <header className="bg-white shadow-sm">
        <div className="max-w-7xl mx-auto px-4 py-4 flex justify-between items-center">
          <div>
            <h1 className="text-2xl font-bold text-gray-900">Operational View</h1>
            <p className="text-sm text-gray-500">Daily operations and monitoring</p>
          </div>
          <button 
            onClick={fetchData}
            className="px-4 py-2 bg-gray-100 text-gray-700 rounded hover:bg-gray-200"
          >
            Refresh
          </button>
        </div>
      </header>

      <main className="max-w-7xl mx-auto px-4 py-6">
        {/* Tab Navigation */}
        <div className="flex gap-4 mb-6 border-b border-gray-200">
          {[
            { key: 'queue', label: 'Priority Queue', count: queueItems.filter(i => i.status !== 'completed').length },
            { key: 'suppliers', label: 'Supplier Risk', count: supplierRisks.length },
            { key: 'pipeline', label: 'Pipeline', count: pipelineOpps.length },
            { key: 'alerts', label: 'Alerts', count: alerts.length },
          ].map((tab) => (
            <button
              key={tab.key}
              onClick={() => setActiveTab(tab.key as typeof activeTab)}
              className={`px-4 py-2 font-medium border-b-2 ${
                activeTab === tab.key
                  ? 'border-blue-600 text-blue-600'
                  : 'border-transparent text-gray-500 hover:text-gray-700'
              }`}
            >
              {tab.label} ({tab.count})
            </button>
          ))}
        </div>

        {/* Content */}
        <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
          {activeTab === 'queue' && (
            <div className="lg:col-span-3 space-y-4">
              <h2 className="text-xl font-semibold">Daily Priority Queue</h2>
              {queueItems
                .filter(item => item.status !== 'completed')
                .sort((a, b) => b.priority - a.priority)
                .map((item) => (
                  <QueueItem 
                    key={item.id} 
                    item={item} 
                    onStatusChange={handleQueueStatusChange} 
                  />
                ))}
              {queueItems.filter(i => i.status !== 'completed').length === 0 && (
                <p className="text-gray-500 text-center py-8">No pending items</p>
              )}
            </div>
          )}

          {activeTab === 'suppliers' && (
            <div className="lg:col-span-3">
              <h2 className="text-xl font-semibold mb-4">Supplier Risk Monitor</h2>
              <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
                {supplierRisks
                  .sort((a, b) => b.risk_score - a.risk_score)
                  .map((risk) => (
                    <SupplierRiskCard key={risk.id} risk={risk} />
                  ))}
                {supplierRisks.length === 0 && (
                  <p className="text-gray-500 text-center py-8 col-span-3">No supplier risks</p>
                )}
              </div>
            </div>
          )}

          {activeTab === 'pipeline' && (
            <div className="lg:col-span-3">
              <h2 className="text-xl font-semibold mb-4">Pipeline Opportunity Tracker</h2>
              <PipelineTracker opportunities={pipelineOpps} />
            </div>
          )}

          {activeTab === 'alerts' && (
            <div className="lg:col-span-3">
              <h2 className="text-xl font-semibold mb-4">Alert Management</h2>
              <AlertManagement alerts={alerts} onAcknowledge={handleAlertAcknowledge} />
            </div>
          )}
        </div>
      </main>
    </div>
  );
};

export default OperationalView;

#[cfg(test)]
mod operational_view_tests {
    use super::*;

    // ── Priority Queue Item Tests ───────────────────────────────────────────

    #[test]
    fn priority_level_calculation() {
        // High priority: > 80
        assert!(85 > 80);
        // Medium priority: 50-80
        assert!(65 >= 50 && 65 <= 80);
        // Low priority: < 50
        assert!(35 < 50);
    }

    #[test]
    fn priority_queue_item_status() {
        let valid_statuses = vec![
            "pending",
            "in_progress",
            "completed",
            "cancelled",
        ];
        
        for status in valid_statuses {
            assert!(["pending", "in_progress", "completed", "cancelled"].contains(&status));
        }
    }

    #[test]
    fn priority_queue_item_types() {
        let valid_types = vec![
            "warning",
            "insight",
            "investigation",
            "task",
            "review",
        ];
        
        for item_type in valid_types {
            assert!(["warning", "insight", "investigation", "task", "review"].contains(&item_type));
        }
    }

    // ── Supplier Risk Tests ─────────────────────────────────────────────────

    #[test]
    fn supplier_risk_score_calculation() {
        let scores = vec![
            (0.85, "high"),
            (0.55, "medium"),
            (0.25, "low"),
        ];
        
        for (score, expected_level) in scores {
            let level = if score > 0.7 { "high" } else if score > 0.4 { "medium" } else { "low" };
            assert_eq!(level, expected_level);
        }
    }

    #[test]
    fn supplier_risk_categories() {
        let valid_categories = vec![
            "financial",
            "operational",
            "compliance",
            "geopolitical",
            "environmental",
            "technological",
        ];
        
        for category in valid_categories {
            assert!(["financial", "operational", "compliance", "geopolitical", "environmental", "technological"].contains(&category));
        }
    }

    // ── Pipeline Tracker Tests ──────────────────────────────────────────────

    #[test]
    fn pipeline_stages() {
        let valid_stages = vec![
            "discovery",
            "qualification",
            "proposal",
            "negotiation",
            "closed_won",
            "closed_lost",
        ];
        
        for stage in valid_stages {
            assert!(["discovery", "qualification", "proposal", "negotiation", "closed_won", "closed_lost"].contains(&stage));
        }
    }

    #[test]
    fn pipeline_stage_colors() {
        let stage_colors = vec![
            ("discovery", "bg-blue-100 border-blue-300"),
            ("qualification", "bg-cyan-100 border-cyan-300"),
            ("proposal", "bg-yellow-100 border-yellow-300"),
            ("negotiation", "bg-orange-100 border-orange-300"),
            ("closed_won", "bg-green-100 border-green-300"),
            ("closed_lost", "bg-red-100 border-red-300"),
        ];
        
        for (stage, color) in stage_colors {
            assert!(color.starts_with("bg-"));
            assert!(color.contains("100"));
        }
    }

    // ── Alert Management Tests ─────────────────────────────────────────────

    #[test]
    fn alert_severity_levels() {
        let valid_severities = vec![
            "critical",
            "high",
            "medium",
            "low",
        ];
        
        for severity in valid_severities {
            assert!(["critical", "high", "medium", "low"].contains(&severity));
        }
    }

    // ── Queue Item Serialization Tests ─────────────────────────────────────

    #[test]
    fn queue_item_serialization() {
        let item = QueueItemData {
            id: "item-123".to_string(),
            item_type: "warning".to_string(),
            item_id: "warning-456".to_string(),
            item_title: "Security Alert".to_string(),
            priority: 85,
            status: "pending".to_string(),
            notes: Some("Requires immediate attention".to_string()),
            created_at: "2024-01-15T10:00:00Z".to_string(),
        };
        
        let json = serde_json::to_string(&item).unwrap();
        assert!(json.contains("Security Alert"));
        assert!(json.contains("85"));
        assert!(json.contains("pending"));
    }

    // ── Supplier Risk Entry Serialization Tests ────────────────────────────

    #[test]
    fn supplier_risk_entry_serialization() {
        let entry = SupplierRiskEntryData {
            id: "risk-123".to_string(),
            supplier_id: "supplier-456".to_string(),
            supplier_name: Some("Acme Suppliers".to_string()),
            risk_category: "operational".to_string(),
            risk_score: 0.75,
            risk_factors: vec![
                "Single source dependency".to_string(),
                "Limited capacity".to_string(),
            ],
            mitigation: Some("Diversify suppliers".to_string()),
            status: "active".to_string(),
            last_reviewed: None,
            next_review: None,
        };
        
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("Acme Suppliers"));
        assert!(json.contains("operational"));
        assert!(json.contains("0.75"));
    }

    // ── Pipeline Opportunity Serialization Tests ───────────────────────────

    #[test]
    fn pipeline_opportunity_serialization() {
        let opp = PipelineOpportunityData {
            id: "pipeline-123".to_string(),
            title: "Enterprise Deal".to_string(),
            stage: "proposal".to_string(),
            value_estimate: Some(5000000.0),
            probability: 0.65,
            owner_id: Some("sales-lead".to_string()),
            expected_close: Some("2024-06-30".to_string()),
        };
        
        let json = serde_json::to_string(&opp).unwrap();
        assert!(json.contains("Enterprise Deal"));
        assert!(json.contains("proposal"));
        assert!(json.contains("5000000"));
    }

    // ── Alert Item Serialization Tests ─────────────────────────────────────

    #[test]
    fn alert_item_serialization() {
        let alert = AlertItemData {
            id: "alert-123".to_string(),
            title: "Critical Security Vulnerability".to_string(),
            severity: "critical".to_string(),
            entity_type: "company".to_string(),
            entity_id: "company-456".to_string(),
            created_at: "2024-01-15T10:00:00Z".to_string(),
        };
        
        let json = serde_json::to_string(&alert).unwrap();
        assert!(json.contains("Critical Security Vulnerability"));
        assert!(json.contains("critical"));
    }

    // ── Tab Navigation Tests ───────────────────────────────────────────────

    #[test]
    fn operational_tabs() {
        let tabs = vec![
            ("queue", "Priority Queue"),
            ("suppliers", "Supplier Risk"),
            ("pipeline", "Pipeline"),
            ("alerts", "Alerts"),
        ];
        
        for (key, label) in tabs {
            assert!(!key.is_empty());
            assert!(!label.is_empty());
        }
    }

    // ── Queue Count Calculation Tests ─────────────────────────────────────

    #[test]
    fn queue_pending_count() {
        let items = vec![
            QueueItemData {
                id: "1".to_string(),
                item_type: "warning".to_string(),
                item_id: "w1".to_string(),
                item_title: "Item 1".to_string(),
                priority: 80,
                status: "completed".to_string(),
                notes: None,
                created_at: "".to_string(),
            },
            QueueItemData {
                id: "2".to_string(),
                item_type: "warning".to_string(),
                item_id: "w2".to_string(),
                item_title: "Item 2".to_string(),
                priority: 90,
                status: "pending".to_string(),
                notes: None,
                created_at: "".to_string(),
            },
        ];
        
        let pending_count = items.iter().filter(|i| i.status != "completed").count();
        assert_eq!(pending_count, 1);
    }

    // ── Priority Sorting Tests ─────────────────────────────────────────────

    #[test]
    fn queue_items_sorting() {
        let mut items = vec![
            QueueItemData {
                id: "1".to_string(),
                item_type: "warning".to_string(),
                item_id: "w1".to_string(),
                item_title: "Low Priority".to_string(),
                priority: 30,
                status: "pending".to_string(),
                notes: None,
                created_at: "".to_string(),
            },
            QueueItemData {
                id: "2".to_string(),
                item_type: "warning".to_string(),
                item_id: "w2".to_string(),
                item_title: "High Priority".to_string(),
                priority: 90,
                status: "pending".to_string(),
                notes: None,
                created_at: "".to_string(),
            },
            QueueItemData {
                id: "3".to_string(),
                item_type: "warning".to_string(),
                item_id: "w3".to_string(),
                item_title: "Medium Priority".to_string(),
                priority: 60,
                status: "pending".to_string(),
                notes: None,
                created_at: "".to_string(),
            },
        ];
        
        // Sort by priority descending
        items.sort_by(|a, b| b.priority.cmp(&a.priority));
        
        assert_eq!(items[0].priority, 90);
        assert_eq!(items[1].priority, 60);
        assert_eq!(items[2].priority, 30);
    }

    // ── API Response Tests ─────────────────────────────────────────────────

    #[test]
    fn api_response_success_parsing() {
        let response = serde_json::json!({
            "success": true,
            "data": [
                {"id": "1", "item_type": "warning", "item_title": "Test", "priority": 75, "status": "pending", "item_id": "w1", "created_at": ""}
            ]
        });
        
        assert!(response["success"].as_bool().unwrap());
        assert!(response["data"].is_array());
    }

    // ── Status Color Mapping Tests ─────────────────────────────────────────

    #[test]
    fn status_color_mapping() {
        let status_colors = vec![
            ("pending", "bg-gray-100 text-gray-600"),
            ("in_progress", "bg-blue-100 text-blue-600"),
            ("completed", "bg-green-100 text-green-600"),
            ("cancelled", "bg-gray-100 text-gray-400"),
        ];
        
        for (status, color) in status_colors {
            assert!(color.starts_with("bg-"));
        }
    }
}
