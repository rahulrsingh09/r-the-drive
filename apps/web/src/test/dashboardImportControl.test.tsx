import React from 'react';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  navigate: vi.fn(),
  mutateAsync: vi.fn(),
}));

vi.mock('@tanstack/react-router', () => ({
  useNavigate: () => mocks.navigate,
}));

vi.mock('@panda/dashboards', () => ({
  importDashboardYaml: () => ({ id: '12345678-aaaa-bbbb-cccc-000000000000', slug: 'trips', name: 'Trips', widgets: [{}, {}] }),
  materializeUserDashboardDraft: (draft: unknown) => draft,
  useCreateDashboard: () => ({ mutateAsync: mocks.mutateAsync, isPending: false }),
}));

import { DashboardImportControl } from '../components/dashboard/DashboardImportControl';

async function chooseFile() {
  const input = screen.getByLabelText('Import from YAML').querySelector('input[type="file"]') as HTMLInputElement;
  const file = new File(['schemaVersion: 2'], 'trips.yaml', { type: 'application/yaml' });
  Object.defineProperty(file, 'text', { value: () => Promise.resolve('schemaVersion: 2') });
  await act(async () => {
    fireEvent.change(input, { target: { files: [file] } });
  });
  await screen.findByRole('heading', { name: 'Import dashboard' });
}

describe('DashboardImportControl', () => {
  beforeEach(() => {
    mocks.navigate.mockReset();
    mocks.mutateAsync.mockReset();
  });

  it('says the dashboard could not be created when the create request fails, and keeps the preview', async () => {
    mocks.mutateAsync.mockRejectedValue(new Error('500 HTTP_500: boom'));
    render(<DashboardImportControl />);
    await chooseFile();

    fireEvent.click(screen.getByRole('button', { name: 'Import copy' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('The dashboard could not be created.');
    expect(screen.getByRole('heading', { name: 'Import dashboard' })).toBeInTheDocument();
    expect(mocks.navigate).not.toHaveBeenCalled();
  });

  it('says the dashboard was created but could not be opened when navigation fails', async () => {
    mocks.mutateAsync.mockResolvedValue({ id: 'dash-1', slug: 'trips-import-12345678' });
    mocks.navigate.mockRejectedValue(new Error('navigation failed'));
    render(<DashboardImportControl />);
    await chooseFile();

    fireEvent.click(screen.getByRole('button', { name: 'Import copy' }));
    expect(await screen.findByRole('alert')).toHaveTextContent(
      'The dashboard was created but could not be opened. Open it from the dashboard library.',
    );
    expect(screen.queryByRole('heading', { name: 'Import dashboard' })).toBeNull();
  });

  it('clears the error on Cancel', async () => {
    mocks.mutateAsync.mockRejectedValue(new Error('failed'));
    render(<DashboardImportControl />);
    await chooseFile();
    fireEvent.click(screen.getByRole('button', { name: 'Import copy' }));
    await screen.findByRole('alert');

    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(screen.queryByRole('heading', { name: 'Import dashboard' })).toBeNull();
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('clears the error when the dialog closes', async () => {
    mocks.mutateAsync.mockRejectedValue(new Error('failed'));
    render(<DashboardImportControl />);
    await chooseFile();
    fireEvent.click(screen.getByRole('button', { name: 'Import copy' }));
    await screen.findByRole('alert');

    fireEvent.keyDown(document, { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('heading', { name: 'Import dashboard' })).toBeNull());
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('gives the import label a visible focus ring equal to the global ring', () => {
    render(<DashboardImportControl />);
    expect(screen.getByLabelText('Import from YAML')).toHaveClass(
      'has-[:focus-visible]:outline-2',
      'has-[:focus-visible]:outline-offset-2',
      'has-[:focus-visible]:outline-accent',
    );
  });
});
