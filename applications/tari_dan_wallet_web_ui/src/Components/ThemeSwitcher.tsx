//  Copyright 2022. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

import { Button } from '@mui/material';
import { IoMoonOutline, IoSunny } from 'react-icons/io5';
import useThemeStore from '../store/themeStore';
import { useTheme } from '@mui/material/styles';

const ThemeSwitcher = () => {
  const { themeMode, setThemeMode } = useThemeStore();
  const theme = useTheme();

  return (
    <Button
      onClick={() => setThemeMode(themeMode === 'light' ? 'dark' : 'light')}
      style={{
        borderRadius: 0,
        color: theme.palette.text.secondary,
        padding: '0.8rem 28px',
        width: '100%',
        justifyContent: 'flex-start',
      }}
      startIcon={themeMode === 'light' ? <IoMoonOutline /> : <IoSunny />}
    >
      <span
        style={{
          marginLeft: '1rem',
          fontSize: '14px',
        }}
      >
        {themeMode === 'light' ? 'Dark Mode' : 'Light Mode'}
      </span>
    </Button>
  );
};

export default ThemeSwitcher;
