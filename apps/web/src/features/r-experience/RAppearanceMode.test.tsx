import React from 'react';
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

const onModeChange = vi.fn();
vi.mock('../../hooks/useThemePreferenceController', () => ({
  useThemePreferenceController: () => ({ mode: 'dark', onModeChange, isPending: false, error: null }),
}));

import { RAppearanceMode } from './RAppearanceMode';

describe('RAppearanceMode', () => {
  it('shows System, Light and Dark with the saved mode checked, and saves a new mode', () => {
    render(<RAppearanceMode />);
    expect(screen.getByRole('radio', { name: 'Dark' })).toHaveAttribute('aria-checked', 'true');
    fireEvent.click(screen.getByRole('radio', { name: 'System' }));
    expect(onModeChange).toHaveBeenCalledWith('system');
    fireEvent.click(screen.getByRole('radio', { name: 'Dark' }));
    expect(onModeChange).toHaveBeenCalledTimes(1);
  });
});
