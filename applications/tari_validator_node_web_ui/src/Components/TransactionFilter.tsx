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

import React, { useState, useRef, useEffect } from 'react';
import TextField from '@mui/material/TextField';
import IconButton from '@mui/material/IconButton';
import FormControl from '@mui/material/FormControl';
import InputLabel from '@mui/material/InputLabel';
import Select from '@mui/material/Select';
import MenuItem from '@mui/material/MenuItem';
import InputAdornment from '@mui/material/InputAdornment';
import SearchIcon from '@mui/icons-material/Search';
import CloseRoundedIcon from '@mui/icons-material/CloseRounded';
import { ITableRecentTransaction } from '../routes/VN/Components/RecentTransactions';

interface ISearchProps {
  recentTransactions: ITableRecentTransaction[];
  setRecentTransactions: (
    recentTransactions: ITableRecentTransaction[]
  ) => void;
  setPage: (page: number) => void;
}

const TransactionFilter: React.FC<ISearchProps> = ({
  recentTransactions,
  setRecentTransactions,
  setPage,
}) => {
  const [formState, setFormState] = useState({ searchValue: '' });
  const [filterBy, setFilterBy] = useState('');
  const [showClearBtn, setShowClearBtn] = useState(false);
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

  // when search input changes and formState has been updated
  useEffect(() => {
    if (formState.searchValue === '') {
      requestSearch(formState.searchValue, filterBy);
      setShowClearBtn(false);
    }
    requestSearch(formState.searchValue, filterBy);
  }, [formState]);

  // once selected filter, focus on input
  useEffect(() => {
    if (filterBy !== '') {
      filterInputRef.current.focus();
    }
  }, [filterBy]);

  // search function
  const requestSearch = (searchedVal: string, filter: string) => {
    const filteredRows = recentTransactions.filter((row) => {
      let result;
      switch (filter) {
        case 'id':
          result = row.id.toLowerCase().includes(searchedVal.toLowerCase());
          break;
        case 'timestamp':
          result = row.timestamp.includes(searchedVal);
          break;
        case 'template':
          console.log('filter by template address');
          break;
        default:
          result = row.id.toLowerCase().includes(searchedVal.toLowerCase());
          break;
      }
      return result;
    });

    // Create a new array that is a copy of the original
    const updatedTransactions = [...recentTransactions];

    // Set the "show" property of all transactions in the copy to false
    updatedTransactions.forEach((transaction) => {
      transaction.show = false;
    });

    // Loop over the filtered array, find the matching object in the
    // original array, and set its "show" property to true
    filteredRows.forEach((filteredRow) => {
      const index = updatedTransactions.findIndex(
        (transaction) => transaction.id === filteredRow.id
      );
      if (index !== -1) {
        updatedTransactions[index].show = true;
      }
    });

    // Update the state with the modified copy of the original array
    setRecentTransactions(updatedTransactions);

    // Set paging to first page
    setPage(0);
  };

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
    setFilterBy('');
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
            <MenuItem value={'template'}>Template Address</MenuItem>
            <MenuItem value={'id'}>Payload ID</MenuItem>
            <MenuItem value={'timestamp'}>Timestamp</MenuItem>
          </Select>
        </FormControl>
        <TextField
          value={formState.searchValue}
          name="searchValue"
          onChange={onTextChange}
          style={{ flexGrow: 1 }}
          inputRef={filterInputRef}
          placeholder="Search for transactions"
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
                <IconButton>
                  {showClearBtn && <CloseRoundedIcon onClick={cancelSearch} />}
                </IconButton>
              </InputAdornment>
            ),
          }}
        />
      </div>
    </>
  );
};

export default TransactionFilter;
