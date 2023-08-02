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

import React, { useState, useRef, useEffect, useCallback } from 'react';
import TextField from '@mui/material/TextField';
import IconButton from '@mui/material/IconButton';
import FormControl from '@mui/material/FormControl';
import InputLabel from '@mui/material/InputLabel';
import Select from '@mui/material/Select';
import MenuItem from '@mui/material/MenuItem';
import InputAdornment from '@mui/material/InputAdornment';
import SearchIcon from '@mui/icons-material/Search';
import CloseRoundedIcon from '@mui/icons-material/CloseRounded';

interface IFilterItems {
  title: string;
  value: string;
  filterFn: (value: string, row: any) => void;
}

interface ISearchProps {
  setPage: (page: number) => void;
  stateObject: any;
  setStateObject: any;
  filterItems: IFilterItems[];
  placeholder: string;
  defaultSearch?: string;
}

// The stateObject being passed to the filter function needs to have an id property for the filter to work

const TransactionFilter: React.FC<ISearchProps> = ({
  setPage,
  stateObject,
  setStateObject,
  filterItems,
  placeholder,
  defaultSearch = 'id',
}) => {
  const [formState, setFormState] = useState({ searchValue: '' });
  const [filterBy, setFilterBy] = useState(defaultSearch);
  const [showClearBtn, setShowClearBtn] = useState(false);
  const [initialUpdate, setInitialUpdate] = useState(true);
  const filterInputRef = useRef<any>(null);

  const onSelectChange = (event: any) => {
    setFilterBy(event.target.value as string);
  };

  // when search input changes
  const onTextChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    // e.preventDefault();
    setFormState({ ...formState, [e.target.name]: e.target.value });
    setShowClearBtn(true);
  };

  // search function
  const requestSearch = useCallback(
    (searchedVal: string, filter: string) => {
      const filteredRows = stateObject.filter((row: any) => {
        const index = filterItems.findIndex((item) => item.value === filter);
        const filterFunction = filterItems[index].filterFn;
        return filterFunction(searchedVal, row);
      });

      // Create a new array that is a copy of the original
      const updatedObject = JSON.parse(JSON.stringify(stateObject));

      // Set the "show" property of all transactions in the copy to false
      updatedObject.forEach((template : any) => {
        template.show = false;
      });

      // Loop over the filtered array, find the matching object in the
      // original array, and set its "show" property to true
      filteredRows.forEach((filteredRow: any) => {
        const index = updatedObject.findIndex(
          (item : any) => item.transaction_hash === filteredRow.transaction_hash
        );
        if (index !== -1) {
          updatedObject[index].show = true;
        }
      });

      // Update the state with the modified copy of the original array, but only
      // when it actually changes, to prevent retriggering redraw
      let changed = updatedObject.length !== stateObject.length;
      if (!changed) {
        for (let i = 0; i < stateObject.length && !changed; ++i) {
          changed ||= stateObject[i] !== updatedObject[i] || stateObject[i]?.["show"] != updatedObject[i]?.["show"];
        }
      }
      if (changed) {
        setStateObject(updatedObject);
      }

      // Set paging to first page
      setPage(0);
    },
    [stateObject, filterItems, setPage, setStateObject]
  );

  // when search input changes and formState has been updated
  useEffect(() => {
    if (formState.searchValue === '') {
      requestSearch(formState.searchValue, filterBy);
      setShowClearBtn(false);
    }
    requestSearch(formState.searchValue, filterBy);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [formState, filterBy]);

  // once selected filter, focus on input

  useEffect(() => {
    if (filterBy !== '' && !initialUpdate) {
      filterInputRef.current.focus();
    } else {
      setInitialUpdate(false);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [filterBy]);

  // search function when enter is pressed
  const confirmSearch = () => {
    requestSearch(formState.searchValue, filterBy);
    if (formState.searchValue !== '') {
      setShowClearBtn(true);
    }
  };

  // clear search
  const cancelSearch = () => {
    setFormState({ ...formState, searchValue: '' });
    setShowClearBtn(false);
    // setFilterBy('');
  };

  return (
    <>
      <div className="flex-container">
        <FormControl>
          <InputLabel>Filter By</InputLabel>
          <Select
            value={filterBy}
            label="Filter By"
            placeholder="Filter By"
            onChange={onSelectChange}
            size="medium"
            name="filterBy"
            style={{ flexGrow: '1', minWidth: '200px' }}
          >
            {filterItems.map((item) => (
              <MenuItem key={item.value} value={item.value}>
                {item.title}
              </MenuItem>
            ))}
          </Select>
        </FormControl>
        <TextField
          value={formState.searchValue}
          name="searchValue"
          onChange={onTextChange}
          style={{ flexGrow: 1 }}
          inputRef={filterInputRef}
          placeholder={placeholder}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              confirmSearch();
            }
          }}
          InputProps={{
            startAdornment: (
              <InputAdornment position="start">
                <SearchIcon />
              </InputAdornment>
            ),
            endAdornment: (
              <InputAdornment position="end">
                {showClearBtn && (
                  <IconButton onClick={cancelSearch} >
                    <CloseRoundedIcon />
                  </IconButton>
                )}
              </InputAdornment>
            ),
          }}
        />
      </div>
    </>
  );
};

export default TransactionFilter;
