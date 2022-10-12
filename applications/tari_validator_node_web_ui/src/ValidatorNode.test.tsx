import React from 'react';
import { render, screen } from '@testing-library/react';
import ValidatorNode from './ValidatorNode';

test('renders learn react link', () => {
  render(<ValidatorNode />);
  const linkElement = screen.getByText(/learn react/i);
  expect(linkElement).toBeInTheDocument();
});
