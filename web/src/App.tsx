import React, { useEffect, useState } from 'react';
import { api } from './api/client';
import { CreateProjectModal } from './components/CreateProjectModal';
import { Header } from './components/Header';
import { ProjectList } from './components/ProjectList';
import { ProjectWorkspace } from './components/ProjectWorkspace';
import { CreateProjectRequest, ProjectSummary, SystemHealth } from './types';

export const App: React.FC = () => {
  const [health, setHealth] = useState<SystemHealth | null>(null);
  const [projects, setProjects] = useState<ProjectSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [selectedProjectId, setSelectedProjectId] = useState<string | null>(null);
  const [isCreateModalOpen, setIsCreateModalOpen] = useState(false);

  const fetchHealth = async () => {
    try {
      const data = await api.getHealth();
      setHealth(data);
    } catch {
      setHealth(null);
    }
  };

  const fetchProjects = async () => {
    try {
      setLoading(true);
      const list = await api.listProjects();
      setProjects(list);
    } catch (err) {
      console.error('Failed to fetch projects', err);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    fetchHealth();
    fetchProjects();

    const interval = setInterval(fetchHealth, 10000);
    return () => clearInterval(interval);
  }, []);

  const handleCreateProject = async (req: CreateProjectRequest) => {
    const newProj = await api.createProject(req);
    await fetchProjects();
    setSelectedProjectId(newProj.id);
  };

  const handleDeleteProject = async (id: string) => {
    await api.deleteProject(id);
    if (selectedProjectId === id) {
      setSelectedProjectId(null);
    }
    await fetchProjects();
  };

  const currentProject = projects.find((p) => p.id === selectedProjectId);

  return (
    <div style={{ minHeight: '100vh', display: 'flex', flexDirection: 'column' }}>
      <Header
        health={health}
        onNewProject={() => setIsCreateModalOpen(true)}
        onBackToDashboard={selectedProjectId ? () => setSelectedProjectId(null) : undefined}
        currentProjectName={currentProject?.name}
      />

      <main style={{ flex: 1 }}>
        {selectedProjectId ? (
          <ProjectWorkspace
            projectId={selectedProjectId}
            onBack={() => {
              setSelectedProjectId(null);
              fetchProjects();
            }}
          />
        ) : (
          <ProjectList
            projects={projects}
            loading={loading}
            onSelectProject={(id) => setSelectedProjectId(id)}
            onDeleteProject={handleDeleteProject}
            onNewProject={() => setIsCreateModalOpen(true)}
          />
        )}
      </main>

      <CreateProjectModal
        isOpen={isCreateModalOpen}
        onClose={() => setIsCreateModalOpen(false)}
        onCreate={handleCreateProject}
      />
    </div>
  );
};
