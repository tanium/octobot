import 'bootstrap/dist/css/bootstrap.min.css';
import 'bootstrap-icons/font/bootstrap-icons.min.css';
import './app.css';

import { mount } from 'svelte';

import App from './App.svelte';

mount(App, { target: document.getElementById('app')! });
